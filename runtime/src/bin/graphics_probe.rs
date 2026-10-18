//! Real D3D8 API test in a hidden, disposable window. The tested Rust runtime
//! is loaded into this process; no other process is opened or modified.
#[cfg(all(windows, target_pointer_width = "32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{
        ffi::c_void,
        io::Write,
        path::{Path, PathBuf},
        ptr,
    };
    use windows_sys::Win32::{
        Foundation::{HWND, RECT},
        System::LibraryLoader::{
            GetModuleHandleW, GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
        },
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, SendMessageW,
            WM_KEYDOWN, WM_LBUTTONDOWN, WNDCLASSW, WS_OVERLAPPEDWINDOW,
        },
    };
    type Object = *mut c_void;
    #[repr(C)]
    struct Parameters {
        width: u32,
        height: u32,
        format: u32,
        count: u32,
        multisample: u32,
        swap: u32,
        window: HWND,
        windowed: i32,
        auto_depth: i32,
        depth_format: u32,
        flags: u32,
        refresh: u32,
        interval: u32,
    }
    struct Com(Object);
    impl Drop for Com {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    let table = *(self.0 as *const *const usize);
                    let release: unsafe extern "system" fn(Object) -> u32 =
                        std::mem::transmute(*table.add(2));
                    release(self.0);
                }
            }
        }
    }
    struct Window(HWND);
    impl Drop for Window {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }
    fn check(code: i32, stage: &str) -> Result<(), String> {
        if code < 0 {
            Err(format!("{stage}: HRESULT {:#010x}", code as u32))
        } else {
            Ok(())
        }
    }
    fn draw_and_verify(device: Object, format: u32) -> Result<(), String> {
        if !matches!(format, 21 | 22) {
            return Err("Pixel test requires A8R8G8B8/X8R8G8B8".into());
        }
        #[repr(C)]
        struct Vertex {
            x: f32,
            y: f32,
            z: f32,
            rhw: f32,
            color: u32,
        }
        #[repr(C)]
        struct LockedRect {
            pitch: i32,
            bits: *const u8,
        }
        let table = unsafe { *(device as *const *const usize) };
        let state: unsafe extern "system" fn(Object, u32, u32) -> i32 =
            unsafe { std::mem::transmute(*table.add(50)) };
        let texture_state: unsafe extern "system" fn(Object, u32, u32, u32) -> i32 =
            unsafe { std::mem::transmute(*table.add(63)) };
        let shader: unsafe extern "system" fn(Object, u32) -> i32 =
            unsafe { std::mem::transmute(*table.add(76)) };
        let begin: unsafe extern "system" fn(Object) -> i32 =
            unsafe { std::mem::transmute(*table.add(34)) };
        let end: unsafe extern "system" fn(Object) -> i32 =
            unsafe { std::mem::transmute(*table.add(35)) };
        let draw: unsafe extern "system" fn(Object, u32, u32, *const Vertex, u32) -> i32 =
            unsafe { std::mem::transmute(*table.add(72)) };
        for (kind, value) in [(7, 0), (22, 1), (27, 0), (137, 0)] {
            check(unsafe { state(device, kind, value) }, "SetRenderState")?;
        }
        for (stage, kind, value) in [(0, 1, 2), (0, 2, 0), (0, 4, 2), (0, 5, 0), (1, 1, 1)] {
            check(
                unsafe { texture_state(device, stage, kind, value) },
                "SetTextureStageState",
            )?;
        }
        check(unsafe { shader(device, 0x44) }, "SetVertexShader(FVF)")?;
        let vertices = [(8.0, 8.0), (100.0, 8.0), (8.0, 100.0)].map(|(x, y)| Vertex {
            x,
            y,
            z: 0.0,
            rhw: 1.0,
            color: 0xffff0000,
        });
        check(unsafe { begin(device) }, "BeginScene")?;
        let drawn = unsafe {
            draw(
                device,
                4,
                1,
                vertices.as_ptr(),
                std::mem::size_of::<Vertex>() as u32,
            )
        };
        let mut overlay = windower_rs::overlay::Frame::new(256, 144)?;
        overlay.rect(120, 8, 30, 30, 0xff00ff00)?;
        overlay.text(120, 50, "RUST", 2, 0xffffffff)?;
        overlay.image(
            120,
            100,
            2,
            2,
            8,
            &[0xff0000ff, 0xffffff00, 0xff00ffff, 0xffff00ff],
        )?;
        let overlay_vertices = overlay.vertices();
        let draw_overlay: unsafe extern "system" fn(
            Object,
            u32,
            u32,
            *const windower_rs::overlay::Vertex,
            u32,
        ) -> i32 = unsafe { std::mem::transmute(*table.add(72)) };
        let overlay_drawn = unsafe {
            draw_overlay(
                device,
                4,
                (overlay_vertices.len() / 3) as u32,
                overlay_vertices.as_ptr(),
                std::mem::size_of::<windower_rs::overlay::Vertex>() as u32,
            )
        };
        let ended = unsafe { end(device) };
        check(drawn, "DrawPrimitiveUP")?;
        check(overlay_drawn, "Overlay DrawPrimitiveUP")?;
        check(ended, "EndScene")?;
        let get_backbuffer: unsafe extern "system" fn(Object, u32, u32, *mut Object) -> i32 =
            unsafe { std::mem::transmute(*table.add(16)) };
        let mut surface = ptr::null_mut();
        check(
            unsafe { get_backbuffer(device, 0, 0, &mut surface) },
            "GetBackBuffer",
        )?;
        if surface.is_null() {
            return Err("No backbuffer surface".into());
        }
        let surface = Com(surface);
        let table = unsafe { *(surface.0 as *const *const usize) };
        let lock: unsafe extern "system" fn(Object, *mut LockedRect, *const RECT, u32) -> i32 =
            unsafe { std::mem::transmute(*table.add(9)) };
        let unlock: unsafe extern "system" fn(Object) -> i32 =
            unsafe { std::mem::transmute(*table.add(10)) };
        let mut locked = LockedRect {
            pitch: 0,
            bits: ptr::null(),
        };
        check(
            unsafe { lock(surface.0, &mut locked, ptr::null(), 0x10) },
            "LockRect",
        )?;
        let observed = if locked.bits.is_null() || locked.pitch < 1024 {
            None
        } else {
            Some(unsafe {
                (
                    ptr::read_unaligned(locked.bits.cast::<u32>()) & 0xffffff,
                    ptr::read_unaligned(
                        locked
                            .bits
                            .add(32 * locked.pitch as usize + 32 * 4)
                            .cast::<u32>(),
                    ) & 0xffffff,
                )
            })
        };
        let overlay_observed = if locked.bits.is_null() || locked.pitch < 1024 {
            None
        } else {
            let pixel = |x: usize, y: usize| unsafe {
                ptr::read_unaligned(
                    locked
                        .bits
                        .add(y * locked.pitch as usize + x * 4)
                        .cast::<u32>(),
                ) & 0xffffff
            };
            Some([
                pixel(125, 15),
                pixel(122, 50),
                pixel(122, 102),
                pixel(130, 102),
                pixel(122, 110),
                pixel(130, 110),
            ])
        };
        check(unsafe { unlock(surface.0) }, "UnlockRect")?;
        if observed != Some((0x223344, 0xff0000)) {
            return Err(format!(
                "GPU pixel readback differs from clear/triangle colors: {observed:?}"
            ));
        }
        if overlay_observed != Some([0x00ff00, 0xffffff, 0x0000ff, 0xffff00, 0x00ffff, 0xff00ff]) {
            return Err(format!(
                "Overlay rectangle/text/image GPU pixels differ: {overlay_observed:?}"
            ));
        }
        Ok(())
    }
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let injected = args.len() == 3 && args[0] == "--injected";
    if args.len() != 2 && !injected {
        return Err(
            "Usage: graphics-probe <Rust runtime DLL> <synthetic windowed settings packet>".into(),
        );
    }
    let (host, counters, lua_counters, input_counters, lua_shutdown, report) = if injected {
        use std::os::windows::ffi::OsStrExt;
        let runtime = Path::new(&args[1])
            .file_name()
            .ok_or("Missing runtime module name")?
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let module = unsafe { GetModuleHandleW(runtime.as_ptr()) };
        if module.is_null() {
            return Err("Runtime was not injected before the child entry point".into());
        }
        let address =
            unsafe { GetProcAddress(module, c"windower_rs_graphics_counter".as_ptr().cast()) }
                .ok_or("No injected graphics counter")?;
        let counters: unsafe extern "system" fn(u32) -> u32 =
            unsafe { std::mem::transmute(address) };
        let lua_address =
            unsafe { GetProcAddress(module, c"windower_rs_lua_counter".as_ptr().cast()) }
                .ok_or("No injected Lua counter")?;
        let lua_counters: unsafe extern "system" fn(u32) -> u32 =
            unsafe { std::mem::transmute(lua_address) };
        let input_address =
            unsafe { GetProcAddress(module, c"windower_rs_input_counter".as_ptr().cast()) }
                .ok_or("No injected input counter")?;
        let input_counters: unsafe extern "system" fn(u32) -> u32 =
            unsafe { std::mem::transmute(input_address) };
        let shutdown_address =
            unsafe { GetProcAddress(module, c"windower_rs_lua_shutdown".as_ptr().cast()) }
                .ok_or("No Lua shutdown export")?;
        let lua_shutdown: unsafe extern "system" fn() -> u32 =
            unsafe { std::mem::transmute(shutdown_address) };
        (
            None,
            counters,
            lua_counters,
            input_counters,
            lua_shutdown,
            Some(PathBuf::from(&args[2])),
        )
    } else {
        let host =
            windower_runtime::probe_host::Host::start(Path::new(&args[0]), Path::new(&args[1]))?;
        let counters = host.counters()?;
        let lua_counters = host.lua_counters()?;
        let input_counters = host.input_counters()?;
        let lua_shutdown = host.lua_shutdown()?;
        (
            Some(host),
            counters,
            lua_counters,
            input_counters,
            lua_shutdown,
            None,
        )
    };
    let _keep_runtime_host_alive = host;
    let trace = |message: &str| {
        if !injected {
            eprintln!("{message}");
        }
    };
    trace("Graphics test: runtime initialized");
    let wide = |value: &str| value.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    // Explicit disposable-test input only. The client runtime does not read
    // this variable; this executable is never used to start a real game.
    let code_fixture = std::env::var_os("WINDOWER_DIAGNOSTIC_FIXTURE");
    if let Some(path) = &code_fixture {
        use std::os::windows::ffi::OsStrExt;
        let path = Path::new(path).canonicalize()?;
        if path.file_name().and_then(|name| name.to_str()) != Some("FFXiMain.dll") {
            return Err("Unexpected code fixture name".into());
        }
        let path = path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let fixture = unsafe {
            LoadLibraryExW(
                path.as_ptr(),
                ptr::null_mut(),
                windows_sys::Win32::System::LibraryLoader::LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR
                    | windows_sys::Win32::System::LibraryLoader::LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
            )
        };
        if fixture.is_null() {
            return Err("Could not load owned code fixture".into());
        }
    }
    let instance = unsafe { GetModuleHandleW(ptr::null()) };
    let class = wide("WindowerRustGraphicsProbe");
    let mut wc: WNDCLASSW = unsafe { std::mem::zeroed() };
    static ORIGINAL_KEYS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    static ORIGINAL_MOUSE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    unsafe extern "system" fn original_window(
        window: HWND,
        message: u32,
        w: usize,
        l: isize,
    ) -> isize {
        if message == WM_KEYDOWN {
            ORIGINAL_KEYS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        if message == WM_LBUTTONDOWN {
            ORIGINAL_MOUSE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        unsafe { DefWindowProcW(window, message, w, l) }
    }
    wc.lpfnWndProc = Some(original_window);
    wc.hInstance = instance;
    wc.lpszClassName = class.as_ptr();
    if unsafe { RegisterClassW(&wc) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let window = Window(unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            320,
            240,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null(),
        )
    });
    if window.0.is_null() {
        return Err(std::io::Error::last_os_error().into());
    }
    let library = wide("d3d8.dll");
    let module = unsafe {
        LoadLibraryExW(
            library.as_ptr(),
            ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    if module.is_null() {
        return Err(std::io::Error::last_os_error().into());
    }
    let create = unsafe { GetProcAddress(module, c"Direct3DCreate8".as_ptr().cast()) }
        .ok_or("No D3D8 factory")?;
    let create: unsafe extern "system" fn(u32) -> Object = unsafe { std::mem::transmute(create) };
    let root = Com(unsafe { create(220) });
    trace("Graphics test: D3D8 factory returned");
    if root.0.is_null() {
        return Err("Direct3DCreate8 returned null".into());
    }
    let table = unsafe { *(root.0 as *const *const usize) };
    #[repr(C)]
    struct DisplayMode {
        width: u32,
        height: u32,
        refresh: u32,
        format: u32,
    }
    let get_mode: unsafe extern "system" fn(Object, u32, *mut DisplayMode) -> i32 =
        unsafe { std::mem::transmute(*table.add(8)) };
    let mut mode = DisplayMode {
        width: 0,
        height: 0,
        refresh: 0,
        format: 0,
    };
    check(
        unsafe { get_mode(root.0, 0, &mut mode) },
        "GetAdapterDisplayMode",
    )?;
    let create_device: unsafe extern "system" fn(
        Object,
        u32,
        u32,
        HWND,
        u32,
        *mut Parameters,
        *mut Object,
    ) -> i32 = unsafe { std::mem::transmute(*table.add(15)) };
    let mut parameters = Parameters {
        width: 256,
        height: 144,
        format: mode.format,
        count: 1,
        multisample: 0,
        swap: 1,
        window: window.0,
        windowed: 1,
        auto_depth: 0,
        depth_format: 0,
        flags: 1,
        refresh: 0,
        interval: 0,
    };
    let mut output = ptr::null_mut();
    check(
        unsafe { create_device(root.0, 0, 1, window.0, 0x20, &mut parameters, &mut output) },
        "CreateDevice",
    )?;
    let device = Com(output);
    trace("Graphics test: device created");
    if device.0.is_null() {
        return Err("CreateDevice succeeded without an object".into());
    }
    let table = unsafe { *(device.0 as *const *const usize) };
    let query: unsafe extern "system" fn(Object, *const c_void, *mut Object) -> i32 =
        unsafe { std::mem::transmute(*table) };
    let unknown: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0xc0, 0, 0, 0, 0, 0, 0, 0x46];
    let mut identity = ptr::null_mut();
    check(
        unsafe { query(device.0, unknown.as_ptr().cast(), &mut identity) },
        "QueryInterface(IUnknown)",
    )?;
    if identity != device.0 {
        return Err("Device IUnknown identity changed".into());
    }
    drop(Com(identity));
    let get_parent: unsafe extern "system" fn(Object, *mut Object) -> i32 =
        unsafe { std::mem::transmute(*table.add(6)) };
    let mut parent = ptr::null_mut();
    check(unsafe { get_parent(device.0, &mut parent) }, "GetDirect3D")?;
    if parent != root.0 {
        return Err("Parent COM identity changed".into());
    }
    drop(Com(parent));
    let clear: unsafe extern "system" fn(Object, u32, *const c_void, u32, u32, f32, u32) -> i32 =
        unsafe { std::mem::transmute(*table.add(36)) };
    let present: unsafe extern "system" fn(
        Object,
        *const c_void,
        *const c_void,
        HWND,
        *const c_void,
    ) -> i32 = unsafe { std::mem::transmute(*table.add(15)) };
    for frame in 0..5u32 {
        trace(&format!("Graphics test: frame {frame}"));
        check(
            unsafe { clear(device.0, 0, ptr::null(), 1, 0xff223344 + frame, 1.0, 0) },
            "Clear",
        )?;
        if frame == 0 {
            draw_and_verify(device.0, mode.format)?;
        }
        check(
            unsafe {
                present(
                    device.0,
                    ptr::null(),
                    ptr::null(),
                    ptr::null_mut(),
                    ptr::null(),
                )
            },
            "Present",
        )?;
    }
    let reset: unsafe extern "system" fn(Object, *mut Parameters) -> i32 =
        unsafe { std::mem::transmute(*table.add(14)) };
    unsafe {
        SendMessageW(window.0, WM_KEYDOWN, 65, (30 << 16) | 1);
        SendMessageW(window.0, WM_LBUTTONDOWN, 1, (7 << 16) | 4);
    }
    parameters.width = 320;
    parameters.height = 180;
    check(unsafe { reset(device.0, &mut parameters) }, "Reset")?;
    check(
        unsafe { clear(device.0, 0, ptr::null(), 1, 0xff445566, 1.0, 0) },
        "Clear after reset",
    )?;
    check(
        unsafe {
            present(
                device.0,
                ptr::null(),
                ptr::null(),
                ptr::null_mut(),
                ptr::null(),
            )
        },
        "Present after reset",
    )?;
    let before = unsafe {
        [
            counters(0),
            counters(1),
            counters(2),
            counters(3),
            counters(4),
        ]
    };
    // Translation providers may unload D3D11/12 when their final COM object is
    // released. Observe modules while the tested device still exists.
    let backend_modules = ["d3d9.dll", "d3d11.dll", "d3d12.dll"]
        .map(|name| unsafe { !GetModuleHandleW(wide(name).as_ptr()).is_null() });
    drop(device);
    trace("Graphics test: device released");
    drop(root);
    trace("Graphics test: root released");
    let after = unsafe { [counters(3), counters(4)] };
    if before != [1, 1, 6, 1, 1] || after != [0, 0] {
        return Err(
            format!("Unexpected D3D8 interception/lifetime counts: {before:?}, {after:?}").into(),
        );
    }
    let hook = wide("Hook.dll");
    let game = wide("FFXiMain.dll");
    let lua = unsafe {
        [
            lua_counters(0),
            lua_counters(1),
            lua_counters(2),
            lua_counters(3),
            lua_counters(4),
            lua_counters(5),
        ]
    };
    if lua[0] != 6 || lua[5] != 0 {
        return Err(format!("Lua frame bridge failed: {lua:?}").into());
    }
    let input = unsafe {
        [
            input_counters(0),
            input_counters(1),
            input_counters(2),
            input_counters(3),
        ]
    };
    let original_keys = ORIGINAL_KEYS.load(std::sync::atomic::Ordering::Relaxed);
    let original_mouse = ORIGINAL_MOUSE.load(std::sync::atomic::Ordering::Relaxed);
    let expected = if lua[1] > 0 {
        [1, 1, 2, 0]
    } else {
        [1, 1, 0, 0]
    };
    if input != expected
        || original_keys != u32::from(lua[1] == 0)
        || original_mouse != u32::from(lua[1] == 0)
    {
        return Err(format!("Input forwarding/blocking mismatch: {input:?}; original {original_keys}/{original_mouse}").into());
    }
    let result = serde_json::json!({"stage":"real_d3d8_interception","pointer_bits":32,"factory_calls":before[0],"devices_created":before[1],"frames_presented":before[2],"device_reset_verified":true,"com_identity_verified":true,"live_objects_after_release":after,"original_hook_loaded":unsafe{!GetModuleHandleW(hook.as_ptr()).is_null()},"game_module_loaded":unsafe{!GetModuleHandleW(game.as_ptr()).is_null()},"injected_before_entry":injected,"input":{"keyboard_messages":input[0],"mouse_messages":input[1],"blocked_messages":input[2],"bound_windows_after_release":input[3],"original_keyboard_messages":original_keys,"original_mouse_messages":original_mouse},"lua":{"frames":lua[0],"addons_loaded":lua[1],"callbacks":lua[2],"commands_queued":lua[3],"chat_messages_queued":lua[4],"errors":lua[5]},"game_integration_complete":false});
    let mut result = result;
    result["external_backend_used"] =
        (unsafe { counters(5) } == 1 && unsafe { counters(6) } == 1).into();
    result["d3d9_module_loaded"] = backend_modules[0].into();
    result["d3d11_module_loaded"] = backend_modules[1].into();
    result["d3d12_module_loaded"] = backend_modules[2].into();
    result["gpu_pixels_verified"] = true.into();
    result["overlay_rectangle_text_image_pixels_verified"] = true.into();
    result["lua"]["windower_commands_queued"] = unsafe { lua_counters(6) }.into();
    result["lua"]["native_commands_queued"] = unsafe { lua_counters(7) }.into();
    result["lua"]["ashita_commands_queued"] = unsafe { lua_counters(8) }.into();
    result["lua"]["luajit_addons_loaded"] = unsafe { lua_counters(9) }.into();
    if unsafe { lua_shutdown() } != 0 || unsafe { lua_shutdown() } != 0 {
        return Err("Addon shutdown did not complete/idempotently repeat".into());
    }
    let shutdown = unsafe {
        [
            lua_counters(1),
            lua_counters(2),
            lua_counters(3),
            lua_counters(5),
            lua_counters(9),
            lua_counters(10),
        ]
    };
    if shutdown != [0, lua[2] + lua[1], lua[3] + lua[1], 0, 0, 1] {
        return Err(format!("Addon unload callbacks/cleanup failed: {shutdown:?}").into());
    }
    let provider_unloaded =
        unsafe { GetModuleHandleW(wide("windower_luajit_provider.dll").as_ptr()).is_null() };
    if !provider_unloaded {
        return Err("LuaJIT provider remained loaded after host shutdown".into());
    }
    result["addon_shutdown"] = serde_json::json!({"addons_remaining":0,"unload_callbacks":lua[1],"unload_commands_queued":lua[1],"errors":0,"provider_unloaded":provider_unloaded,"idempotent":true});
    if let Some(path) = report {
        if code_fixture.is_some() {
            use std::os::windows::ffi::OsStrExt;
            let name = Path::new(&args[1])
                .file_name()
                .ok_or("Missing runtime name")?
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>();
            let module = unsafe { GetModuleHandleW(name.as_ptr()) };
            let address = unsafe {
                GetProcAddress(
                    module,
                    c"windower_rs_wait_client_diagnostic".as_ptr().cast(),
                )
            }
            .ok_or("Missing client diagnostic waiter")?;
            let wait: unsafe extern "system" fn(u32) -> u32 =
                unsafe { std::mem::transmute(address) };
            if unsafe { wait(10000) } != 2 {
                return Err("Client diagnostic worker did not save its report".into());
            }
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(serde_json::to_string_pretty(&result)?.as_bytes())?;
    } else {
        println!("{result}");
    }
    Ok(())
}

#[cfg(not(all(windows, target_pointer_width = "32")))]
fn main() {
    eprintln!("Compile for i686-pc-windows-msvc");
    std::process::exit(1);
}
