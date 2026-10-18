//! Owned fixture only. Loads no game and calls no game function.
#[cfg(all(windows, target_pointer_width = "32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{
        io::Write,
        os::windows::ffi::OsStrExt,
        path::{Path, PathBuf},
    };
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.len() != 3 || args[0] != "--injected" {
        return Err("Use only the suspended owned fixture launcher".into());
    }
    let root = PathBuf::from(
        std::env::var_os("WINDOWER_LIFECYCLE_FIXTURE_ROOT").ok_or("Missing owned fixture root")?,
    );
    let name = Path::new(&args[1])
        .file_name()
        .ok_or("Missing runtime name")?
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let module = unsafe { GetModuleHandleW(name.as_ptr()) };
    if module.is_null() {
        return Err("Fixture runtime was not injected before entry".into());
    }
    let symbol = |name: &std::ffi::CStr| {
        unsafe { GetProcAddress(module, name.as_ptr().cast()) }
            .ok_or("Missing lifecycle fixture export")
    };
    let tick: unsafe extern "system" fn() -> u32 =
        unsafe { std::mem::transmute(symbol(c"windower_rs_lua_tick")?) };
    let reload: unsafe extern "system" fn(*const u8, usize) -> u32 =
        unsafe { std::mem::transmute(symbol(c"windower_rs_lua_reload")?) };
    let index: unsafe extern "system" fn(*const u8, usize) -> u32 =
        unsafe { std::mem::transmute(symbol(c"windower_rs_lua_command_index")?) };
    let counter: unsafe extern "system" fn(u32) -> u32 =
        unsafe { std::mem::transmute(symbol(c"windower_rs_lua_counter")?) };
    let shutdown: unsafe extern "system" fn() -> u32 =
        unsafe { std::mem::transmute(symbol(c"windower_rs_lua_shutdown")?) };
    let position = |text: &str| unsafe { index(text.as_ptr(), text.len()) };
    let file = root.join("addons/ReloadA/ReloadA.lua");
    if unsafe { tick() } != 0 || unsafe { counter(1) } != 2 || unsafe { counter(5) } != 0 {
        return Err("Initial fixture load failed".into());
    }
    if position("A old frame 1") == u32::MAX || position("B frame 1") == u32::MAX {
        return Err("Initial frame dispatch failed".into());
    }
    std::fs::write(&file, "this is invalid Lua !!!")?;
    if unsafe { reload(b"ReloadA".as_ptr(), 7) } != 2 || unsafe { tick() } != 0 {
        return Err("Failed source was not rejected".into());
    }
    if position("A old frame 2") == u32::MAX
        || position("B frame 2") == u32::MAX
        || position("A old unload") != u32::MAX
        || root.join("finalized-A.txt").exists()
        || unsafe { counter(11) } != 0
    {
        return Err("Failed reload damaged the original VM or its neighbor".into());
    }
    std::fs::write(&file, std::fs::read(root.join("replacement-A.lua"))?)?;
    if unsafe { reload(b"reloada".as_ptr(), 7) } != 0 || unsafe { counter(11) } != 1 {
        return Err("Selective reload failed".into());
    }
    if std::fs::read_to_string(root.join("finalized-A.txt"))? != "old\n" {
        return Err("Old VM did not finalize exactly once".into());
    }
    if position("A old unload") >= position("A new load") {
        return Err("Unload/load action order was not preserved".into());
    }
    if unsafe { tick() } != 0
        || position("A new frame 1") == u32::MAX
        || position("B frame 3") == u32::MAX
        || position("A old frame 3") != u32::MAX
        || unsafe { counter(1) } != 2
        || unsafe { counter(5) } != 0
    {
        return Err("Replacement state, neighbor continuity or callback cleanup failed".into());
    }
    let foreign = unsafe { counter(9) };
    if unsafe { reload(b"missing".as_ptr(), 7) } != 2
        || unsafe { reload(b"../bad".as_ptr(), 6) } != 2
    {
        return Err("Invalid reload selection was admitted".into());
    }
    if unsafe { shutdown() } != 0 || unsafe { shutdown() } != 0 || unsafe { counter(1) } != 0 {
        return Err("Final shutdown failed".into());
    }
    let commands = unsafe { counter(3) };
    if unsafe { tick() } != 0
        || unsafe { counter(3) } != commands
        || unsafe { reload(b"ReloadA".as_ptr(), 7) } != 2
    {
        return Err("Stopped host restarted or accepted a reload".into());
    }
    if std::fs::read_to_string(root.join("finalized-A.txt"))? != "old\nnew\n"
        || std::fs::read_to_string(root.join("finalized-B.txt"))? != "B\n"
    {
        return Err("VM finalization was missing or duplicated".into());
    }
    let provider_name = "windower_luajit_provider.dll"
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let provider_unloaded = unsafe { GetModuleHandleW(provider_name.as_ptr()).is_null() };
    if !provider_unloaded {
        return Err("Foreign VM/provider remained loaded after shutdown".into());
    }
    let result = serde_json::json!({"stage":"owned_addon_reload_lifecycle","failed_reload_preserved_original":true,
        "reloads":1,"foreign_addons":foreign,"old_vm_finalized":true,"neighbor_continuity":true,
        "old_callbacks_removed":true,"unload_before_load_actions":true,"shutdown_idempotent":true,
        "all_vms_finalized_once":true,"provider_unloaded":provider_unloaded,"game_functions_called":false});
    let mut report = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&args[2])?;
    report.write_all(&serde_json::to_vec_pretty(&result)?)?;
    Ok(())
}
#[cfg(not(all(windows, target_pointer_width = "32")))]
fn main() {
    eprintln!("Build for i686-pc-windows-msvc");
    std::process::exit(1);
}
