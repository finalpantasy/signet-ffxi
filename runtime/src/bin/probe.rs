//! Isolated native ABI probe: loads only our Rust DLL into this test process.
//! No injection, FFXI, original DLL execution, registry, settings or addon writes.
#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{
        env,
        ffi::c_void,
        fs,
        path::PathBuf,
        ptr, thread,
        time::{Duration, Instant},
    };
    use windower_rs::protocol::{SETTINGS_BYTES, decode_settings};
    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
        },
        System::{
            LibraryLoader::{GetProcAddress, LoadLibraryW},
            Memory::{
                CreateFileMappingW, FILE_MAP_WRITE, MapViewOfFile, PAGE_READWRITE, UnmapViewOfFile,
            },
            Threading::{CreateEventW, WaitForSingleObject},
        },
    };
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("Usage: runtime-probe <Rust DLL> <synthetic settings packet>".into());
    }
    let dll = PathBuf::from(&args[0]).canonicalize()?;
    let bytes = fs::read(&args[1])?;
    let decoded = decode_settings(&bytes)?;
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let (mapping_name, event_name) = windower_runtime::mapping_names();
    let mapping_name = wide(&mapping_name);
    let event_name = wide(&event_name);
    let mapping = unsafe {
        CreateFileMappingW(
            INVALID_HANDLE_VALUE,
            ptr::null(),
            PAGE_READWRITE,
            0,
            SETTINGS_BYTES as u32,
            mapping_name.as_ptr(),
        )
    };
    if mapping.is_null() {
        return Err(std::io::Error::last_os_error().into());
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            CloseHandle(mapping);
        }
        return Err("A Windower settings mapping already exists; probe refused to alter it".into());
    }
    let event = unsafe { CreateEventW(ptr::null(), 0, 0, event_name.as_ptr()) };
    if event.is_null() {
        unsafe {
            CloseHandle(mapping);
        }
        return Err(std::io::Error::last_os_error().into());
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            CloseHandle(event);
            CloseHandle(mapping);
        }
        return Err("A Windower injection event already exists; probe refused to use it".into());
    }
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let view = unsafe { MapViewOfFile(mapping, FILE_MAP_WRITE, 0, 0, SETTINGS_BYTES) };
        if view.Value.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), view.Value.cast::<u8>(), SETTINGS_BYTES);
            UnmapViewOfFile(view);
        }
        let dll_name = dll.as_os_str().to_string_lossy();
        let name = wide(&dll_name);
        let module = unsafe { LoadLibraryW(name.as_ptr()) };
        if module.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        // Keep the module loaded until process exit: its bootstrap worker must
        // never execute after FreeLibrary. This probe owns its entire process.
        let status_address =
            unsafe { GetProcAddress(module, c"windower_rs_runtime_status".as_ptr().cast()) }
                .ok_or("Missing Rust status export")?;
        let status: unsafe extern "system" fn() -> u32 = unsafe {
            std::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                unsafe extern "system" fn() -> u32,
            >(status_address)
        };
        if unsafe { WaitForSingleObject(event, 5000) } != WAIT_OBJECT_0 {
            return Err(
                format!("Bootstrap handshake failed; status {}", unsafe { status() }).into(),
            );
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        while unsafe { status() } == 1 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        if unsafe { status() } != 2 {
            return Err("Rust runtime did not complete bootstrap".into());
        }
        println!(
            "{}",
            serde_json::json!({"stage":"bootstrap_only", "settings_bytes":bytes.len(), "profile":decoded.profile_name, "rust_runtime_status":2,"pointer_bits":std::mem::size_of::<*mut c_void>() * 8,"game_hooks_implemented":false})
        );
        Ok(())
    })();
    unsafe {
        CloseHandle(event);
        CloseHandle(mapping);
    }
    result
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Windows only");
    std::process::exit(1);
}
