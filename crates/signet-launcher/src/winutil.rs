//! Small Win32 helpers: SHA-256, a registry read, process lookups and ShellExecute. No process is ever found or killed by image name alone.

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::Cryptography::{BCRYPT_SHA256_ALG_HANDLE, BCryptHash};
use windows::Win32::System::Diagnostics::ToolHelp::*;
use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, SW_SHOWNORMAL, WM_CLOSE};
use windows::core::BOOL;
use windows::core::{HSTRING, PCWSTR};

pub fn sha256_hex(data: &[u8]) -> Result<String, String> {
    let mut out = [0u8; 32];
    let st = unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, data, &mut out) };
    if st.0 != 0 {
        return Err(format!("BCryptHash failed (0x{:08x})", st.0));
    }
    Ok(out.iter().map(|b| format!("{b:02x}")).collect())
}

/// Full image path of a running process, if it can be opened for limited query.
pub fn image_path(pid: u32) -> Option<PathBuf> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = vec![0u16; 32768];
        let mut len = buf.len() as u32;
        let r = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(h);
        r.ok()?;
        Some(PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    let n = |p: &Path| p.to_string_lossy().replace('/', "\\").to_ascii_lowercase();
    n(a) == n(b)
}

/// PIDs whose image is exactly `exe` (compared by full path, like win_launch.find_client).
pub fn pids_for_exe(exe: &Path) -> Vec<u32> {
    let file = exe.file_name().map(|f| f.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut more = Process32FirstW(snap, &mut e).is_ok();
        while more {
            let len = e.szExeFile.iter().position(|c| *c == 0).unwrap_or(e.szExeFile.len());
            let name = String::from_utf16_lossy(&e.szExeFile[..len]).to_ascii_lowercase();
            if name == file && image_path(e.th32ProcessID).is_some_and(|p| same_path(&p, exe)) {
                out.push(e.th32ProcessID);
            }
            more = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

/// An owned process handle (from ShellExecuteEx) that can be polled for exit.
pub struct ProcHandle(HANDLE);
unsafe impl Send for ProcHandle {}

impl ProcHandle {
    pub fn pid(&self) -> u32 {
        unsafe { GetProcessId(self.0) }
    }
    /// None while running, Some(exit code) after exit.
    pub fn poll(&self) -> Option<u32> {
        let mut code = 0u32;
        unsafe { GetExitCodeProcess(self.0, &mut code).ok()? };
        if code == 259 { None } else { Some(code) }
    }
}

impl Drop for ProcHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// A `REG_SZ` value under HKEY_LOCAL_MACHINE (read only).
pub fn reg_string_hklm(key: &str, value: &str) -> Option<String> {
    let (k, v) = (HSTRING::from(key), HSTRING::from(value));
    let mut buf = vec![0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let r = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, &k, &v, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut size)) };
    if r.is_err() {
        return None;
    }
    let len = (size as usize / 2).min(buf.len());
    let s = String::from_utf16_lossy(&buf[..len]).trim_end_matches('\0').to_string();
    (!s.is_empty()).then_some(s)
}

/// ShellExecuteEx `open` with no arguments, like double-clicking the program. A program whose
/// manifest needs administrator rights (pol.exe) makes Windows ask. Error 1223 = canceled.
pub fn shell_open(exe: &Path, dir: &Path) -> Result<ProcHandle, String> {
    let file = HSTRING::from(exe.as_os_str());
    let dir = HSTRING::from(dir.as_os_str());
    let verb = HSTRING::from("open");
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpDirectory: PCWSTR(dir.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.map_err(|e| {
        if e.code().0 as u32 == 0x800704C7 {
            "Windows approval was canceled.".to_string()
        } else {
            format!("Could not start {}: {e}", exe.display())
        }
    })?;
    if info.hProcess.is_invalid() {
        return Err("Windows started PlayOnline without a process handle.".into());
    }
    Ok(ProcHandle(info.hProcess))
}

/// Posts WM_CLOSE to every visible top-level window of `pid` (what its close button does).
/// Returns how many windows were asked to close.
pub fn close_windows_of(pid: u32) -> usize {
    unsafe extern "system" fn each(hwnd: HWND, lp: LPARAM) -> BOOL {
        let st = unsafe { &mut *(lp.0 as *mut (u32, usize)) };
        let mut owner = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut owner)) };
        if owner == st.0 && unsafe { IsWindowVisible(hwnd) }.as_bool() && unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) }.is_ok() {
            st.1 += 1;
        }
        BOOL(1)
    }
    let mut st = (pid, 0usize);
    let _ = unsafe { EnumWindows(Some(each), LPARAM(&mut st as *mut _ as isize)) };
    st.1
}

/// Opens a folder in Explorer (fixed paths only; the UI never passes a path).
pub fn open_folder(dir: &Path) -> Result<(), String> {
    if !dir.is_dir() {
        return Err(format!("{} is not a folder", dir.display()));
    }
    std::process::Command::new("explorer.exe").arg(dir).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vector() {
        assert_eq!(sha256_hex(b"abc").unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn own_process_is_found_by_full_path_only() {
        let me = std::env::current_exe().unwrap();
        assert!(pids_for_exe(&me).contains(&std::process::id()));
        assert!(pids_for_exe(&me.with_file_name("not-here.exe")).is_empty());
    }

    #[test]
    fn registry_read() {
        assert!(reg_string_hklm(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion", "ProductName").is_some());
        assert!(reg_string_hklm(r"SOFTWARE\Signet\NoSuchKey", "x").is_none());
    }
}
