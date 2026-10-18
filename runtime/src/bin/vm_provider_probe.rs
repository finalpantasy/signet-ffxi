//! Verify both Lua engines in one disposable process. No game or original core.
#[cfg(all(windows, target_pointer_width = "32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use windower_rs::commands::Dispatcher;
    use windower_rs::{platform::Ecosystem, vm_protocol::*};
    use windower_runtime::game_bridge::NativeInput;
    use windower_runtime::{
        lua::{Addon, HostAction},
        vm_client::Provider,
    };
    fn check(response: Response) -> Result<Response, String> {
        if !response.ok {
            return Err(response.error.unwrap_or("Unknown VM failure".into()));
        }
        if !response.errors.is_empty()
            || response
                .callbacks
                .iter()
                .any(|callback| callback.error.is_some())
        {
            return Err(format!("Unexpected callback errors: {response:?}"));
        }
        Ok(response)
    }
    static INPUTS: std::sync::Mutex<Vec<(i32, Vec<u8>)>> = std::sync::Mutex::new(Vec::new());
    unsafe extern "C" fn input_fixture(text: *const std::ffi::c_char, mode: i32) -> i32 {
        INPUTS.lock().unwrap().push((
            mode,
            unsafe { std::ffi::CStr::from_ptr(text) }
                .to_bytes()
                .to_vec(),
        ));
        73
    }
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.len() != 1 {
        return Err("Usage: vm-provider-probe <trusted LuaJIT provider DLL>".into());
    }
    let legacy = Addon::from_source(
        "puc",
        "C:/Test/",
        "C:/Test/puc/",
        r#"
        assert(_VERSION == 'Lua 5.1' and jit == nil)
        windower.register_event('load', function() windower.send_command('puc alive') end)
    "#,
    )?;
    let jit_version;
    {
        let provider =
            std::sync::Arc::new(unsafe { Provider::open(std::path::Path::new(&args[0])) }?);
        assert_eq!(provider.resource_counts()?, (0, 0));
        let source = r#"
            assert(_VERSION == 'Lua 5.1' and type(jit) == 'table')
            local ffi = require('ffi')
            assert(ffi.sizeof('void*') == 4)
            ffi.cdef[[size_t strlen(const char*);]]
            assert(tonumber(ffi.C.strlen('Ashita')) == 6)
            ffxi.commands.queue(jit.version)
            local data = ffi.new('uint8_t[3]', {65, 0, 255})
            ashita.events.register('load', 'load', function()
                AshitaCore:GetChatManager():QueueCommand(1, '/echo LuaJIT FFI')
                return ffi.string(data, 3), 42, nil
            end)
            ashita.events.register('d3d_present', 'frame', function()
                AshitaCore:GetChatManager():QueueCommand(1, '/echo LuaJIT frame')
            end)
            windower.register_event('binary', function(value, blocked)
                assert(value == 'A' .. string.char(0, 255) and not blocked)
                return true
            end)
            windower.register_event('threaded', function(index) return index end)
            windower.register_event('input_bridge', function()
                windower.chat.input('/echo literal;semicolon')
                ffxi.commands.input('/echo native')
                windower.send_command('input /echo console;wait 0;input /echo resumed')
                AshitaCore:GetChatManager():QueueCommand(2, '/echo macro')
            end)
            coroutine.schedule(function()
                ffxi.commands.queue('scheduled')
                coroutine.sleep(0)
                ffxi.commands.queue('resumed')
            end, 0)
        "#;
        let created = check(provider.request(Request::new(Operation::Create {
            name: "jit".into(),
            runtime_path: "C:/Test/".into(),
            addon_path: "C:/Test/jit/".into(),
            source: source.into(),
            ecosystem: Ecosystem::Ashita4,
        }))?)?;
        let handle = created
            .handle
            .ok_or("Provider did not return an addon handle")?;
        match created.actions.as_slice() {
            [Action::NativeCommand { bytes }] => {
                jit_version = String::from_utf8(bytes.clone())?;
                assert!(jit_version.starts_with("LuaJIT"));
            }
            other => return Err(format!("Unexpected creation actions: {other:?}").into()),
        }
        assert_eq!(provider.resource_counts()?, (1, 0));
        let loaded = check(provider.request(Request::new(Operation::Emit {
            handle,
            event: "load".into(),
            arguments: vec![],
        }))?)?;
        assert_eq!(loaded.callbacks.len(), 1);
        assert_eq!(
            loaded.callbacks[0].values,
            vec![
                Value::Bytes(vec![65, 0, 255]),
                Value::Number(42.0),
                Value::Nil
            ]
        );
        assert_eq!(
            loaded.actions,
            vec![Action::AshitaCommand {
                mode: 1,
                bytes: b"/echo LuaJIT FFI".to_vec()
            }]
        );
        for index in 1..=2 {
            let frame = check(provider.request(Request::new(Operation::Frame { handle, index }))?)?;
            assert_eq!(frame.callbacks.len(), 1);
            assert_eq!(
                frame.actions,
                vec![Action::AshitaCommand {
                    mode: 1,
                    bytes: b"/echo LuaJIT frame".to_vec()
                }]
            );
        }
        for command in ["scheduled", "resumed"] {
            let polled = check(provider.request(Request::new(Operation::Poll {
                handle,
                seconds: 0.0,
            }))?)?;
            assert_eq!(
                polled.actions,
                vec![Action::NativeCommand {
                    bytes: command.as_bytes().to_vec()
                }]
            );
        }
        let blocked = check(provider.request(Request::new(Operation::EmitBlocking {
            handle,
            event: "binary".into(),
            arguments: vec![Value::Bytes(vec![65, 0, 255]), Value::Boolean(false)],
            blocked_at: 1,
            blocked: false,
        }))?)?;
        assert!(blocked.blocked);
        assert_eq!(blocked.callbacks.len(), 1);
        let inputs = check(provider.request(Request::new(Operation::Emit {
            handle,
            event: "input_bridge".into(),
            arguments: vec![],
        }))?)?;
        assert_eq!(inputs.actions.len(), 4);
        let mut dispatcher = Dispatcher::default();
        for action in inputs.actions {
            dispatcher.enqueue(1, action, std::time::Duration::ZERO)?;
        }
        let mut backend = unsafe { NativeInput::from_validated_function(input_fixture) };
        let first = dispatcher.pump(std::time::Duration::ZERO, &mut backend)?;
        assert_eq!(first.delivered, 3);
        assert!(first.failures.is_empty());
        let second = dispatcher.pump(std::time::Duration::ZERO, &mut backend)?;
        assert_eq!(second.delivered, 2);
        assert!(second.failures.is_empty());
        assert_eq!(
            *INPUTS.lock().unwrap(),
            vec![
                (1, b"/echo literal;semicolon".to_vec()),
                (1, b"/echo native".to_vec()),
                (1, b"/echo console".to_vec()),
                (1, b"/echo resumed".to_vec()),
                (2, b"/echo macro".to_vec()),
            ]
        );
        assert_eq!(dispatcher.pending_batches(), 0);
        let workers = (0..2)
            .map(|worker| {
                let provider = provider.clone();
                std::thread::spawn(move || -> Result<(), String> {
                    for index in 0..32 {
                        let number = f64::from(worker * 32 + index);
                        let response = check(provider.request(Request::new(Operation::Emit {
                            handle,
                            event: "threaded".into(),
                            arguments: vec![Value::Number(number)],
                        }))?)?;
                        assert_eq!(response.callbacks.len(), 1);
                        assert_eq!(response.callbacks[0].values, vec![Value::Number(number)]);
                    }
                    Ok(())
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().map_err(|_| "Provider worker panicked")??;
        }
        assert_eq!(provider.resource_counts()?, (1, 0));
        let invalid =
            provider.request(Request::new(Operation::TakeActions { handle: u64::MAX }))?;
        assert!(!invalid.ok && invalid.error.unwrap().contains("Unknown addon"));
        let bad_version = provider.request(Request {
            version: 99,
            operation: Operation::TakeActions { handle },
        })?;
        assert!(!bad_version.ok && bad_version.error.unwrap().contains("version"));
        check(provider.request(Request::new(Operation::Destroy { handle }))?)?;
        assert_eq!(provider.resource_counts()?, (0, 0));
    }
    assert!(
        legacy
            .emit("load", mlua::MultiValue::new())
            .iter()
            .all(|(_, result)| result.is_ok())
    );
    assert_eq!(
        legacy.take_actions(),
        vec![HostAction::Command(b"puc alive".to_vec())]
    );
    let wide = |value: &str| value.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let module_loaded = |name: &str| unsafe {
        !windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(wide(name).as_ptr()).is_null()
    };
    let provider_name = std::path::Path::new(&args[0])
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Invalid provider name")?;
    assert!(
        !module_loaded(provider_name),
        "Provider DLL remained loaded after full cleanup"
    );
    println!(
        "{}",
        serde_json::json!({"checks_passed":true,"stage":"isolated_vm_provider",
        "pointer_bits":32,"core_engine":"PUC Lua 5.1","provider_engine":jit_version,
        "ffi_call_verified":true,"binary_values_verified":true,"scheduling_verified":true,
        "blocking_verified":true,"invalid_handles_rejected":true,"resources_after_release":[0,0],
        "cross_thread_callbacks_verified":64,"provider_unloaded":true,
        "reference_input_abi_verified":true,"synthetic_input_deliveries":5,"live_game_calls_verified":false,
        "puc_survives_provider_unload":true,"original_hook_loaded":module_loaded("Hook.dll"),
        "original_ashita_core_loaded":module_loaded("Ashita.dll"),"game_module_loaded":module_loaded("FFXiMain.dll"),
        "game_integration_complete":false})
    );
    Ok(())
}

#[cfg(not(all(windows, target_pointer_width = "32")))]
fn main() {
    eprintln!("Compile for i686-pc-windows-msvc");
    std::process::exit(1);
}
