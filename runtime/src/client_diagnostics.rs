//! Explicit first-client diagnostic. No Lua source or game functions execute.
//! A worker scans only this process after FFXiMain reaches a real Present.
use std::{
    io::Write,
    sync::atomic::{AtomicBool, Ordering},
    sync::{Condvar, Mutex},
};
use windows_sys::Win32::System::{
    LibraryLoader::GetModuleHandleW,
    Threading::{GetCurrentProcessId, GetCurrentThreadId},
};
static STARTED: AtomicBool = AtomicBool::new(false);
static FINISHED: (Mutex<Option<bool>>, Condvar) = (Mutex::new(None), Condvar::new());
fn finish(success: bool) {
    *FINISHED.0.lock().unwrap_or_else(|error| error.into_inner()) = Some(success);
    FINISHED.1.notify_all();
}
pub fn wait(timeout_ms: u32) -> u32 {
    let state = FINISHED.0.lock().unwrap_or_else(|error| error.into_inner());
    let (state, _) = FINISHED
        .1
        .wait_timeout_while(
            state,
            std::time::Duration::from_millis(u64::from(timeout_ms.min(60000))),
            |state| state.is_none(),
        )
        .unwrap_or_else(|error| error.into_inner());
    match *state {
        Some(true) => 2,
        Some(false) => 3,
        None => 0,
    }
}

pub fn on_present() {
    let Some(path) = crate::native::options().and_then(|options| options.client_diagnostic.clone())
    else {
        return;
    };
    if STARTED.load(Ordering::Acquire) {
        return;
    }
    let wide = |name: &str| name.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    if unsafe { GetModuleHandleW(wide("FFXiMain.dll").as_ptr()).is_null() } {
        return;
    }
    if STARTED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let process_id = unsafe { GetCurrentProcessId() };
    let render_thread_id = unsafe { GetCurrentThreadId() };
    let worker = std::thread::Builder::new().name("ffxi-code-diagnostic".into()).spawn(move || {
        let success = std::panic::catch_unwind(move || {
        let result = windower_rs::client_code::scan_loaded_client(process_id);
        let loaded = |name: &str| unsafe { !GetModuleHandleW(wide(name).as_ptr()).is_null() };
        let (scan, error, _probe_scan) = match result {
            Ok(report) => (serde_json::to_value(&report).ok(), None, Some(report)),
            Err(error) => (None, Some(error), None),
        };
        #[cfg(feature="client-windowing")]
        let display_overrides=Some(crate::display::overrides());
        #[cfg(not(feature="client-windowing"))]
        let display_overrides=None::<u32>;
        let report = serde_json::json!({"stage":"new_client_code_diagnostic","process_id":process_id,
            "render_thread_id":render_thread_id,"scan":scan,"error":error,
            "original_hook_loaded":loaded("Hook.dll"),"original_lua_core_loaded":loaded("LuaCore.dll"),
            "original_ashita_core_loaded":loaded("Ashita.dll"),"graphics":{"factory_calls":crate::graphics::counter(0),
            "devices_created":crate::graphics::counter(1),"frames_presented":crate::graphics::counter(2)},
            "virtual_display_overrides":display_overrides,"controller_discovery":crate::controller_discovery::observe(),"game_functions_called":false,"addons_loaded":0,"game_integration_complete":false});
        let Ok(bytes) = serde_json::to_vec_pretty(&report) else { return false; };
        let saved = std::fs::OpenOptions::new().write(true).create_new(true).open(path).and_then(|mut file| file.write_all(&bytes)).is_ok();
        finish(saved);
        #[cfg(feature = "game-thread-probe")]
        if saved && let Some(scan) = _probe_scan.as_ref()
            && let Some(config) = crate::native::options().and_then(|options| options.game_thread_probe.as_ref()) {
            crate::game_thread_probe::run(config, scan);
        }
        saved
        }).unwrap_or(false);
        finish(success);
    });
    if worker.is_err() {
        finish(false);
    }
}
