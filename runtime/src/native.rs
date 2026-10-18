use std::{
    ffi::c_void,
    panic::catch_unwind,
    ptr,
    sync::{
        OnceLock,
        atomic::{AtomicU32, Ordering},
    },
};
use windower_rs::protocol::{DecodedSettings, SETTINGS_BYTES, decode_settings};
use windower_rs::runtime_options::{OPTIONS_BYTES, RuntimeOptions};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, HINSTANCE},
    System::{
        LibraryLoader::DisableThreadLibraryCalls,
        Memory::{FILE_MAP_READ, MapViewOfFile, OpenFileMappingW, UnmapViewOfFile},
        Threading::{CreateThread, EVENT_MODIFY_STATE, OpenEventW, SetEvent},
    },
};

static STATUS: AtomicU32 = AtomicU32::new(0);
static SETTINGS: OnceLock<DecodedSettings> = OnceLock::new();
static OPTIONS: OnceLock<RuntimeOptions> = OnceLock::new();

#[cfg(any(feature = "isolated-probe", feature = "game-thread-probe"))]
pub(crate) fn settings() -> Option<&'static DecodedSettings> {
    SETTINGS.get()
}
#[cfg(any(feature = "isolated-probe", feature = "client-diagnostics"))]
pub(crate) fn options() -> Option<&'static RuntimeOptions> {
    OPTIONS.get()
}
#[cfg(feature = "client-diagnostics")]
#[unsafe(no_mangle)]
pub extern "system" fn windower_rs_client_diagnostics_version() -> u32 {
    1
}
#[cfg(all(target_pointer_width = "32", feature = "client-windowing"))]
#[unsafe(no_mangle)]
pub extern "system" fn windower_rs_virtual_display_version() -> u32 {
    1
}
#[cfg(all(target_pointer_width = "32", feature = "client-windowing"))]
#[unsafe(no_mangle)]
pub extern "system" fn windower_rs_virtual_display_overrides() -> u32 {
    crate::display::overrides()
}
#[cfg(feature = "game-thread-probe")]
#[unsafe(no_mangle)]
pub extern "system" fn windower_rs_game_thread_probe_version() -> u32 {
    1
}
#[cfg(all(target_pointer_width = "32", feature = "client-diagnostics"))]
#[unsafe(no_mangle)]
pub extern "system" fn windower_rs_wait_client_diagnostic(timeout_ms: u32) -> u32 {
    crate::client_diagnostics::wait(timeout_ms)
}

#[cfg(feature = "isolated-probe")]
#[unsafe(no_mangle)]
pub extern "system" fn windower_rs_lua_counter(which: u32) -> u32 {
    crate::frame_host::counter(which)
}
/// Stop fixture addon dispatch and release VMs outside Windows' loader lock.
/// 0 = stopped, 1 = active dispatch/load (retry), 2 = caught Rust panic.
#[cfg(feature = "isolated-probe")]
#[unsafe(no_mangle)]
pub extern "system" fn windower_rs_lua_shutdown() -> u32 {
    match catch_unwind(crate::frame_host::shutdown) {
        Ok(Ok(())) => 0,
        Ok(Err(_)) => 1,
        Err(_) => 2,
    }
}
/// Reload one already selected addon in an isolated host. 0=success, 1=busy,
/// 2=invalid/unloaded/source failure, 3=Rust panic. This is not a game API.
/// # Safety
/// The name pointer must remain readable for `length` bytes during this call.
#[cfg(feature = "isolated-probe")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn windower_rs_lua_reload(name: *const u8, length: usize) -> u32 {
    if name.is_null() || length == 0 || length > 256 {
        return 2;
    }
    match catch_unwind(|| {
        let name = std::str::from_utf8(unsafe { std::slice::from_raw_parts(name, length) })
            .map_err(|_| "Invalid UTF-8 name".to_string())?;
        crate::frame_host::reload(name)
    }) {
        Ok(Ok(())) => 0,
        Ok(Err(error)) if error.contains("busy") => 1,
        Ok(Err(_)) => 2,
        Err(_) => 3,
    }
}
#[cfg(feature = "isolated-probe")]
#[unsafe(no_mangle)]
pub extern "system" fn windower_rs_lua_tick() -> u32 {
    match catch_unwind(crate::frame_host::on_frame) {
        Ok(()) => 0,
        Err(_) => 2,
    }
}
/// # Safety
/// The byte pointer must be readable for `length` bytes during this call.
#[cfg(feature = "isolated-probe")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn windower_rs_lua_command_index(
    bytes: *const u8,
    length: usize,
) -> u32 {
    if bytes.is_null() || length > 65536 {
        return u32::MAX;
    }
    catch_unwind(|| {
        crate::frame_host::command_index(unsafe { std::slice::from_raw_parts(bytes, length) })
    })
    .unwrap_or(u32::MAX)
}

#[cfg(all(target_pointer_width = "32", feature = "isolated-probe"))]
#[unsafe(no_mangle)]
pub extern "system" fn windower_rs_input_counter(which: u32) -> u32 {
    crate::input::counter(which)
}

pub fn runtime_status() -> u32 {
    STATUS.load(Ordering::Acquire)
}

pub fn mapping_names() -> (String, String) {
    windower_rs::protocol::runtime_mapping_names(unsafe {
        windows_sys::Win32::System::Threading::GetCurrentProcessId()
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn windower_rs_runtime_status() -> u32 {
    runtime_status()
}

#[unsafe(no_mangle)]
pub extern "system" fn windower_rs_protocol_version() -> u32 {
    0x0407_0903
}

#[cfg(target_pointer_width = "32")]
#[unsafe(no_mangle)]
pub extern "system" fn windower_rs_graphics_counter(which: u32) -> u32 {
    crate::graphics::counter(which)
}

// The stock Hook also starts a worker on process attach (REA ev_92f6f3b1...).
// All shared memory, allocation and runtime work happens on that worker after
// the Windows loader has released its lock. DllMain never waits for it.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllMain(module: HINSTANCE, reason: u32, _: *mut c_void) -> i32 {
    if reason == 1 {
        unsafe {
            DisableThreadLibraryCalls(module);
        }
        STATUS.store(1, Ordering::Release);
        let thread = unsafe {
            CreateThread(
                ptr::null(),
                0,
                Some(bootstrap),
                ptr::null_mut(),
                0,
                ptr::null_mut(),
            )
        };
        if thread.is_null() {
            STATUS.store(3, Ordering::Release);
            return 0;
        }
        unsafe {
            CloseHandle(thread);
        }
    }
    1
}

unsafe extern "system" fn bootstrap(_: *mut c_void) -> u32 {
    let outcome = catch_unwind(initialize);
    match outcome {
        Ok(Ok(())) => {
            STATUS.store(2, Ordering::Release);
            0
        }
        _ => {
            STATUS.store(3, Ordering::Release);
            1
        }
    }
}

fn initialize() -> Result<(), String> {
    let (mapping_name, signal_name) = mapping_names();
    let name = wide(&mapping_name);
    let mapping = OwnedHandle(unsafe { OpenFileMappingW(FILE_MAP_READ, 0, name.as_ptr()) });
    if mapping.0.is_null() {
        return Err("Missing launch settings mapping".into());
    }
    let view = unsafe { MapViewOfFile(mapping.0, FILE_MAP_READ, 0, 0, SETTINGS_BYTES) };
    if view.Value.is_null() {
        return Err("Could not map launch settings".into());
    }
    let result = unsafe {
        let bytes = std::slice::from_raw_parts(view.Value.cast::<u8>(), SETTINGS_BYTES);
        decode_settings(bytes)
    };
    unsafe {
        UnmapViewOfFile(view);
    }
    SETTINGS
        .set(result?)
        .map_err(|_| "Runtime already initialized")?;
    let option_name = wide(&windower_rs::runtime_options::mapping_name(unsafe {
        windows_sys::Win32::System::Threading::GetCurrentProcessId()
    }));
    let option_mapping =
        OwnedHandle(unsafe { OpenFileMappingW(FILE_MAP_READ, 0, option_name.as_ptr()) });
    let options = if option_mapping.0.is_null() {
        if unsafe { windows_sys::Win32::Foundation::GetLastError() }
            != windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND
        {
            return Err("Could not open runtime options".into());
        }
        RuntimeOptions::default()
    } else {
        let view = unsafe { MapViewOfFile(option_mapping.0, FILE_MAP_READ, 0, 0, OPTIONS_BYTES) };
        if view.Value.is_null() {
            return Err("Could not map runtime options".into());
        }
        let result = RuntimeOptions::decode(unsafe {
            std::slice::from_raw_parts(view.Value.cast::<u8>(), OPTIONS_BYTES)
        });
        unsafe {
            UnmapViewOfFile(view);
        }
        result?
    };
    OPTIONS
        .set(options)
        .map_err(|_| "Runtime options already initialized")?;
    if OPTIONS.get().is_some_and(|options| options.virtual_display) {
        #[cfg(all(target_pointer_width = "32", feature = "client-windowing"))]
        unsafe {
            crate::display::install(SETTINGS.get().ok_or("Missing display settings")?)?;
        }
        #[cfg(not(all(target_pointer_width = "32", feature = "client-windowing")))]
        return Err("Virtual display requires the client-windowing build".into());
    }
    #[cfg(not(feature = "game-thread-probe"))]
    if OPTIONS
        .get()
        .is_some_and(|options| options.game_thread_probe.is_some())
    {
        return Err("Game thread probe requires its dedicated runtime build".into());
    }
    if OPTIONS
        .get()
        .is_some_and(|options| options.client_diagnostic.is_some())
    {
        #[cfg(not(feature = "client-diagnostics"))]
        return Err("Client diagnostics require the dedicated runtime build".into());
        #[cfg(feature = "client-diagnostics")]
        if SETTINGS.get().is_some_and(|settings| {
            !settings.init_addons.is_empty() || !settings.init_plugins.is_empty()
        }) {
            return Err("Client diagnostic cannot load legacy addons/plugins".into());
        }
    }
    #[cfg(target_pointer_width = "32")]
    unsafe {
        crate::graphics::install(&OPTIONS.get().ok_or("Missing runtime options")?.graphics)?;
    }
    // Signal only the bootstrap handshake. This is NOT a declaration that game
    // hooks or native plugin compatibility have been implemented.
    let signal = wide(&signal_name);
    let event = OwnedHandle(unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, signal.as_ptr()) });
    if event.0.is_null() {
        return Err("Missing injection event".into());
    }
    if unsafe { SetEvent(event.0) } == 0 {
        return Err("Could not signal bootstrap".into());
    }
    Ok(())
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
struct OwnedHandle(HANDLE);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}
