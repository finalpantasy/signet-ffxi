#[cfg(all(windows, target_pointer_width = "32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    // This dedicated, short-lived child is the explicit native-code boundary.
    // It never initializes a plugin against the game or the original runtime.
    let mut result = if args.len() == 1 {
        let plugin = unsafe {
            windower_runtime::plugin::Plugin::open_metadata(std::path::Path::new(&args[0]))
        }?;
        let metadata = plugin.metadata();
        serde_json::json!({
        "ecosystem":"windower4",
        "stage":"plugin_factory_and_metadata_only", "name":metadata.name,
        "author":metadata.author,"interface_version":format!("{:#010x}", metadata.interface_version),
        "pointer_bits":32,"initialized":false,"shutdown_called":false,
        })
    } else if args.len() == 2 && args[0] == "--ashita" {
        let plugin = unsafe {
            windower_runtime::ashita_plugin::Plugin::open_metadata(std::path::Path::new(&args[1]))
        }?;
        let metadata = plugin.metadata();
        serde_json::json!({"stage":"plugin_factory_and_metadata_only", "ecosystem":"ashita4",
            "name":metadata.name,"author":metadata.author,"description":metadata.description,
            "link":metadata.link,"version":metadata.version,"interface_version":metadata.interface_version,
            "priority":metadata.priority,"flags":metadata.flags,"pointer_bits":32,
            "initialized":false,"shutdown_called":false})
    } else {
        return Err("Usage: plugin-probe [--ashita] <trusted plugin DLL>".into());
    };
    let wide = |value: &str| value.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let hook = wide("Hook.dll");
    let game = wide("FFXiMain.dll");
    result["original_hook_loaded"] = unsafe {
        !windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(hook.as_ptr()).is_null()
    }
    .into();
    result["game_module_loaded"] = unsafe {
        !windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(game.as_ptr()).is_null()
    }
    .into();
    result["original_ashita_core_loaded"] = unsafe {
        !windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(wide("Ashita.dll").as_ptr())
            .is_null()
    }
    .into();
    println!("{result}");
    Ok(())
}

#[cfg(not(all(windows, target_pointer_width = "32")))]
fn main() {
    eprintln!("Compile this probe for i686-pc-windows-msvc");
    std::process::exit(1);
}
