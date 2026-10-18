//! Separate-process Windows control test. Never loads or hooks FFXI.
use std::{
    io::Write,
    path::Path,
    time::{Duration, Instant},
};
use windows_sys::Win32::System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId};
fn save(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    f.write_all(&serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())
}
fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        return Err("Usage: profile-control-probe LAB-CONFIG.json NEW-RESULT.json".into());
    }
    let config = std::fs::read(&args[0]).map_err(|e| e.to_string())?;
    if config.len() > 65536 {
        return Err("Oversized synthetic config".into());
    }
    let lab: windower_rs::runtime_options::LabTest =
        serde_json::from_slice(&config).map_err(|e| e.to_string())?;
    for name in ["FFXiMain.dll", "Hook.dll", "LuaCore.dll", "Ashita.dll"] {
        let wide: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
        if unsafe { !GetModuleHandleW(wide.as_ptr()).is_null() } {
            return Err("Game/runtime module in synthetic process".into());
        }
    }
    let mut session = windower_runtime::profile_control::Session::new(&lab)?;
    let owner = Path::new(&lab.request)
        .parent()
        .ok_or("Missing private directory")?
        .join("synthetic-owner.json");
    save(
        &owner,
        &serde_json::json!({"process_id":std::process::id(),"owner_thread_id":unsafe{GetCurrentThreadId()},"no_game_modules":true}),
    )?;
    let deadline = Instant::now() + Duration::from_secs(125);
    while Instant::now() < deadline {
        if let Some(mut result) = session.tick()? {
            result["synthetic"] = true.into();
            result["no_game_modules"] = true.into();
            return save(Path::new(&args[1]), &result);
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    Err("Synthetic control process deadline expired".into())
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            std::process::ExitCode::FAILURE
        }
    }
}
