//! New-process startup/injection. This module cannot attach to an existing PID.
//! Verification children are bounded and terminated on failure. Interactive
//! clients are detached only after successful injection/bootstrap/resume.
use crate::protocol::{SETTINGS_BYTES, decode_settings};
use crate::runtime_options::{OPTIONS_BYTES, RuntimeOptions};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{Read, Seek, SeekFrom},
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr,
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
        WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    System::{
        Diagnostics::Debug::{
            SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX, SetErrorMode, WriteProcessMemory,
        },
        LibraryLoader::{GetModuleHandleW, GetProcAddress},
        Memory::{
            CreateFileMappingW, FILE_MAP_WRITE, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE,
            MapViewOfFile, PAGE_READWRITE, UnmapViewOfFile, VirtualAllocEx, VirtualFreeEx,
        },
        Threading::{
            CREATE_NO_WINDOW, CREATE_SUSPENDED, CreateEventW, CreateProcessW, CreateRemoteThread,
            GetExitCodeProcess, GetExitCodeThread, PROCESS_INFORMATION, ResumeThread, STARTUPINFOW,
            TerminateProcess, WaitForSingleObject,
        },
    },
};

pub struct ChildResult {
    pub process_id: u32,
    pub exit_code: u32,
}
pub struct StartedClient {
    child: OwnedChild,
    pub process_id: u32,
}
impl StartedClient {
    /// Terminate only this process created by our launcher, for explicit user
    /// closure or a failed disposable startup test. No PID attachment exists.
    pub fn terminate(&self) -> Result<(), String> {
        if unsafe { TerminateProcess(self.child.process.0, 0xe0000001) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if unsafe { WaitForSingleObject(self.child.process.0, 5000) } != WAIT_OBJECT_0 {
            return Err("Owned client did not terminate".into());
        }
        Ok(())
    }
    pub fn wait_for_exit(&self, timeout_ms: u32) -> Result<Option<u32>, String> {
        match unsafe { WaitForSingleObject(self.child.process.0, timeout_ms) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut code = 0;
                if unsafe { GetExitCodeProcess(self.child.process.0, &mut code) } == 0 {
                    return Err(std::io::Error::last_os_error().to_string());
                }
                Ok(Some(code))
            }
            _ => Err(std::io::Error::last_os_error().to_string()),
        }
    }
}
/// Start only a newly created client. Dropping the result closes our handles
/// and leaves a successfully started interactive process running.
pub fn launch_interactive_client(
    executable: &Path,
    runtime: &Path,
    packet: &[u8],
    arguments: &[OsString],
    options: &RuntimeOptions,
) -> Result<StartedClient, String> {
    let (mut child, process_id) =
        start_child(executable, runtime, packet, arguments, options, true)?;
    child.terminate_on_drop = false;
    Ok(StartedClient { child, process_id })
}

/// Create a fresh, initially suspended i686 process, load the specified i686
/// runtime, wait for readiness, then resume and wait for that child to exit.
/// No existing game or process is opened. The caller selects trusted binaries.
/// The shared kernel32 LoadLibrary address follows the same i686 Windows
/// startup mechanism as the reference launcher; cross-bitness is rejected.
pub fn run_isolated_child(
    executable: &Path,
    runtime: &Path,
    packet: &[u8],
    arguments: &[OsString],
) -> Result<ChildResult, String> {
    run_isolated_child_with_options(
        executable,
        runtime,
        packet,
        arguments,
        &RuntimeOptions::default(),
    )
}

pub fn run_isolated_child_with_options(
    executable: &Path,
    runtime: &Path,
    packet: &[u8],
    arguments: &[OsString],
    options: &RuntimeOptions,
) -> Result<ChildResult, String> {
    let (child, process_id) = start_child(executable, runtime, packet, arguments, options, false)?;
    if unsafe { WaitForSingleObject(child.process.0, 30000) } != WAIT_OBJECT_0 {
        return Err("Owned test child did not exit within 30 seconds".into());
    }
    let mut exit_code = 0;
    if unsafe { GetExitCodeProcess(child.process.0, &mut exit_code) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    if exit_code != 0 {
        return Err(format!("Owned test child failed: {exit_code:#010x}"));
    }
    Ok(ChildResult {
        process_id,
        exit_code,
    })
}
fn start_child(
    executable: &Path,
    runtime: &Path,
    packet: &[u8],
    arguments: &[OsString],
    options: &RuntimeOptions,
    interactive: bool,
) -> Result<(OwnedChild, u32), String> {
    decode_settings(packet)?;
    let option_bytes = options.encode()?;
    options.verify_external()?;
    let executable = executable.canonicalize().map_err(|e| e.to_string())?;
    let runtime = runtime.canonicalize().map_err(|e| e.to_string())?;
    validate_i686(&executable, false)?;
    validate_i686(&runtime, true)?;
    // canonicalize() produces \\?\ paths. Keep those for validation, but
    // legacy POL expects ordinary DOS/UNC names in its process parameters.
    let application = process_path(executable.as_os_str())?;
    let mut command = quote_units(&application[..application.len() - 1]);
    // Match Injector.Launch: argv[0] is quoted even when it has no spaces.
    if command.first() != Some(&34) {
        command.insert(0, 34);
        command.push(34);
    }
    for arg in arguments {
        command.push(32);
        let units = arg.encode_wide().collect::<Vec<_>>();
        if units.contains(&0) {
            return Err("NUL in child argument".into());
        }
        command.extend(quote_units(&units));
    }
    command.push(0);
    if command.len() > 32767 {
        return Err("Windows command line exceeds 32767 UTF-16 units".into());
    }
    let current_dir = process_path(
        executable
            .parent()
            .ok_or("Missing child directory")?
            .as_os_str(),
    )?;
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    if !interactive {
        startup.dwFlags = 1;
        startup.wShowWindow = 0;
    }
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    unsafe {
        SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX);
    }
    if unsafe {
        CreateProcessW(
            application.as_ptr(),
            command.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            CREATE_SUSPENDED | if interactive { 0 } else { CREATE_NO_WINDOW },
            ptr::null(),
            current_dir.as_ptr(),
            &startup,
            &mut info,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let child = OwnedChild {
        process: Handle(info.hProcess),
        thread: Handle(info.hThread),
        terminate_on_drop: true,
    };
    let (mapping_name, signal_name) = crate::protocol::runtime_mapping_names(info.dwProcessId);
    let mapping_name = wide(OsStr::new(&mapping_name))?;
    let signal_name = wide(OsStr::new(&signal_name))?;
    let mapping = Handle(unsafe {
        CreateFileMappingW(
            INVALID_HANDLE_VALUE,
            ptr::null(),
            PAGE_READWRITE,
            0,
            SETTINGS_BYTES as u32,
            mapping_name.as_ptr(),
        )
    });
    if mapping.0.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        return Err("Child mapping already exists".into());
    }
    let signal = Handle(unsafe { CreateEventW(ptr::null(), 0, 0, signal_name.as_ptr()) });
    if signal.0.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        return Err("Child signal already exists".into());
    }
    let view = unsafe { MapViewOfFile(mapping.0, FILE_MAP_WRITE, 0, 0, SETTINGS_BYTES) };
    if view.Value.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    unsafe {
        ptr::copy_nonoverlapping(packet.as_ptr(), view.Value.cast(), packet.len());
        UnmapViewOfFile(view);
    }
    let option_name = wide(OsStr::new(&crate::runtime_options::mapping_name(
        info.dwProcessId,
    )))?;
    let option_mapping = Handle(unsafe {
        CreateFileMappingW(
            INVALID_HANDLE_VALUE,
            ptr::null(),
            PAGE_READWRITE,
            0,
            OPTIONS_BYTES as u32,
            option_name.as_ptr(),
        )
    });
    if option_mapping.0.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        return Err("Child options mapping already exists".into());
    }
    let view = unsafe { MapViewOfFile(option_mapping.0, FILE_MAP_WRITE, 0, 0, OPTIONS_BYTES) };
    if view.Value.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    unsafe {
        ptr::copy_nonoverlapping(option_bytes.as_ptr(), view.Value.cast(), option_bytes.len());
        UnmapViewOfFile(view);
    }
    let library = wide(runtime.as_os_str())?;
    let bytes = library.len() * 2;
    let remote = unsafe {
        VirtualAllocEx(
            child.process.0,
            ptr::null(),
            bytes,
            MEM_COMMIT | MEM_RESERVE,
            PAGE_READWRITE,
        )
    };
    if remote.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // On an error, the owned-child guard terminates this newly created process
    // and the OS reclaims its memory. Never free a path while a loader thread
    // might still be reading it.
    let mut written = 0;
    if unsafe {
        WriteProcessMemory(
            child.process.0,
            remote,
            library.as_ptr().cast(),
            bytes,
            &mut written,
        )
    } == 0
        || written != bytes
    {
        return Err("Could not write runtime path to the owned child".into());
    }
    let kernel = wide(OsStr::new("kernel32.dll"))?;
    let module = unsafe { GetModuleHandleW(kernel.as_ptr()) };
    let loader = unsafe { GetProcAddress(module, c"LoadLibraryW".as_ptr().cast()) }
        .ok_or("Missing LoadLibraryW")?;
    let start = unsafe {
        std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            unsafe extern "system" fn(*mut std::ffi::c_void) -> u32,
        >(loader)
    };
    let loader_thread = Handle(unsafe {
        CreateRemoteThread(
            child.process.0,
            ptr::null(),
            0,
            Some(start),
            remote,
            0,
            ptr::null_mut(),
        )
    });
    if loader_thread.0.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    if unsafe { WaitForSingleObject(loader_thread.0, 20000) } != WAIT_OBJECT_0 {
        return Err("Owned child loader did not finish within 20 seconds".into());
    }
    let mut module_result = 0;
    if unsafe { GetExitCodeThread(loader_thread.0, &mut module_result) } == 0 || module_result == 0
    {
        return Err("Owned child could not load the Rust runtime".into());
    }
    unsafe {
        VirtualFreeEx(child.process.0, remote, 0, MEM_RELEASE);
    }
    if unsafe { WaitForSingleObject(signal.0, 20000) } != WAIT_OBJECT_0 {
        return Err("Rust runtime did not signal readiness within 20 seconds".into());
    }
    if unsafe { ResumeThread(child.thread.0) } == u32::MAX {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok((child, info.dwProcessId))
}

fn validate_i686(path: &Path, dll: bool) -> Result<(), String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut dos = [0u8; 64];
    file.read_exact(&mut dos).map_err(|e| e.to_string())?;
    if &dos[..2] != b"MZ" {
        return Err("Selected program is not a PE image".into());
    }
    let offset = u32::from_le_bytes(dos[60..64].try_into().map_err(|_| "Bad PE offset")?);
    file.seek(SeekFrom::Start(offset.into()))
        .map_err(|e| e.to_string())?;
    let mut header = [0u8; 24];
    file.read_exact(&mut header).map_err(|e| e.to_string())?;
    if &header[..4] != b"PE\0\0" || u16::from_le_bytes([header[4], header[5]]) != 0x14c {
        return Err("Only i686 PE images are supported".into());
    }
    if (u16::from_le_bytes([header[22], header[23]]) & 0x2000 != 0) != dll {
        return Err("Expected an executable and runtime DLL respectively".into());
    }
    Ok(())
}
fn wide(value: &OsStr) -> Result<Vec<u16>, String> {
    let mut units = value.encode_wide().collect::<Vec<_>>();
    if units.contains(&0) {
        return Err("NUL in Windows path".into());
    }
    units.push(0);
    Ok(units)
}
fn process_path(value: &OsStr) -> Result<Vec<u16>, String> {
    let units = value.encode_wide().collect::<Vec<_>>();
    let mut units = normal_process_path(&units)?;
    units.push(0);
    Ok(units)
}
pub fn normal_process_path(units: &[u16]) -> Result<Vec<u16>, String> {
    if units.contains(&0) {
        return Err("NUL in process path".into());
    }
    let prefix = [92, 92, 63, 92];
    if units.starts_with(&prefix) {
        let rest = &units[4..];
        if rest.starts_with(&[85, 78, 67, 92]) {
            return Ok([&[92, 92][..], &rest[4..]].concat());
        }
        if rest.len() >= 3
            && ((65..=90).contains(&rest[0]) || (97..=122).contains(&rest[0]))
            && rest[1..3] == [58, 92]
        {
            return Ok(rest.to_vec());
        }
        return Err("Unsupported extended process path".into());
    }
    Ok(units.to_vec())
}
pub fn quote_units(value: &[u16]) -> Vec<u16> {
    if !value.is_empty() && !value.iter().any(|c| matches!(*c, 9 | 32 | 34)) {
        return value.to_vec();
    }
    let mut result = vec![34];
    let mut slashes = 0;
    for &unit in value {
        match unit {
            92 => slashes += 1,
            34 => {
                result.extend(std::iter::repeat_n(92, slashes * 2 + 1));
                result.push(34);
                slashes = 0;
            }
            _ => {
                result.extend(std::iter::repeat_n(92, slashes));
                result.push(unit);
                slashes = 0;
            }
        }
    }
    result.extend(std::iter::repeat_n(92, slashes * 2));
    result.push(34);
    result
}
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}
struct OwnedChild {
    process: Handle,
    thread: Handle,
    terminate_on_drop: bool,
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.terminate_on_drop
            && unsafe { WaitForSingleObject(self.process.0, 0) } == WAIT_TIMEOUT
        {
            unsafe {
                TerminateProcess(self.process.0, 0xe0000001);
                WaitForSingleObject(self.process.0, 5000);
            }
        }
    }
}
