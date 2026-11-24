//! Checks that run before anything is written: where the install is, that it
//! looks like FINAL FANTASY XI, and that the game is not running.

use std::path::{Path, PathBuf};

use crate::error::{PatchError, Result};

pub const REGISTRY_KEY: &str = r"SOFTWARE\WOW6432Node\PlayOnlineUS\InstallFolder";
/// Registry value names under [`REGISTRY_KEY`].
pub const VALUE_FFXI: &str = "0001";
pub const VALUE_VIEWER: &str = "1000";

/// Executables that mean the game or its launcher is up. Matched on the file
/// name of each running process's image path.
pub const GAME_IMAGES: &[&str] = &["pol.exe", "polboot.exe", "ffxi-boot.exe", "playonlineviewer.exe"];

/// The FFXI install folder from the registry.
pub fn registry_install() -> Result<PathBuf> {
    read_install_value(VALUE_FFXI)
        .map(PathBuf::from)
        .ok_or_else(|| PatchError::failed("FINAL FANTASY XI is not in the registry. Use --install <folder>."))
}

/// The PlayOnline Viewer folder from the registry, if present.
pub fn registry_viewer() -> Option<PathBuf> {
    read_install_value(VALUE_VIEWER).map(PathBuf::from)
}

/// Basic shape of an FFXI install folder.
pub fn check_install_shape(root: &Path) -> Result<()> {
    let missing: Vec<&str> =
        ["VTABLE.DAT", "FTABLE.DAT", "ROM"].into_iter().filter(|n| !root.join(n).exists()).collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(PatchError::refused(format!(
            "{} is not a FINAL FANTASY XI folder (missing {})",
            root.display(),
            missing.join(", ")
        )))
    }
}

/// A running process that blocks patching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocker {
    pub pid: u32,
    pub image: String,
}

/// True when `image` is one of the game executables, or lives under the
/// folder that holds the install (the SquareEnix folder, which also holds
/// PlayOnline Viewer).
pub fn is_blocking(image: &str, install: &Path) -> bool {
    let lower = image.replace('/', "\\").to_ascii_lowercase();
    let name = lower.rsplit('\\').next().unwrap_or(&lower);
    if GAME_IMAGES.contains(&name) {
        return true;
    }
    let base = install.parent().unwrap_or(install);
    let mut prefix = base.to_string_lossy().replace('/', "\\").to_ascii_lowercase();
    if !prefix.ends_with('\\') {
        prefix.push('\\');
    }
    prefix.len() > 3 && lower.starts_with(&prefix)
}

/// Refuse while the game or anything from its folder is running.
pub fn refuse_if_running(install: &Path) -> Result<()> {
    let hits: Vec<Blocker> = running_images().into_iter().filter(|b| is_blocking(&b.image, install)).collect();
    if hits.is_empty() {
        return Ok(());
    }
    let list: Vec<String> = hits.iter().map(|b| format!("{} (pid {})", b.image, b.pid)).collect();
    Err(PatchError::refused(format!("close the game first: {}", list.join(", "))))
}

#[cfg(windows)]
fn read_install_value(name: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    let key: Vec<u16> = REGISTRY_KEY.encode_utf16().chain([0]).collect();
    let value: Vec<u16> = name.encode_utf16().chain([0]).collect();
    let mut buf = vec![0u16; 1024];
    let mut bytes = (buf.len() * 2) as u32;
    // SAFETY: valid NUL-terminated strings and a buffer whose byte size is passed.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if status != 0 {
        return None;
    }
    let len = (bytes as usize / 2).min(buf.len());
    let s = String::from_utf16_lossy(&buf[..len]);
    let s = s.trim_end_matches('\0').trim().to_string();
    (!s.is_empty()).then_some(s)
}

#[cfg(not(windows))]
fn read_install_value(_name: &str) -> Option<String> {
    None
}

/// Every process's image path, or its exe name when the path cannot be read.
/// Read-only: processes are opened for limited query access only.
#[cfg(windows)]
pub fn running_images() -> Vec<Blocker> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };

    let mut out = Vec::new();
    // SAFETY: standard Toolhelp snapshot walk; every handle opened is closed.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut more = Process32FirstW(snap, &mut entry) != 0;
        while more {
            let pid = entry.th32ProcessID;
            let name_len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            let mut image = String::from_utf16_lossy(&entry.szExeFile[..name_len]);
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if !h.is_null() {
                let mut buf = [0u16; 1024];
                let mut n = buf.len() as u32;
                if QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut n) != 0 {
                    image = String::from_utf16_lossy(&buf[..n as usize]);
                }
                CloseHandle(h);
            }
            out.push(Blocker { pid, image });
            more = Process32NextW(snap, &mut entry) != 0;
        }
        CloseHandle(snap);
    }
    out
}

#[cfg(not(windows))]
pub fn running_images() -> Vec<Blocker> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocking_rule() {
        let install = Path::new(r"E:\Games\SquareEnix\FINAL FANTASY XI");
        assert!(is_blocking(r"C:\Other\pol.exe", install));
        assert!(is_blocking(r"C:\Other\POLBOOT.EXE", install));
        assert!(is_blocking(r"e:\games\squareenix\PlayOnlineViewer\something.exe", install));
        assert!(is_blocking(r"E:/Games/SquareEnix/FINAL FANTASY XI/x.exe", install));
        assert!(!is_blocking(r"C:\Windows\explorer.exe", install));
        assert!(!is_blocking(r"E:\Games\SquareEnixOther\a.exe", install));
        assert!(!is_blocking("explorer.exe", install));
    }

    #[test]
    fn process_list_is_readable() {
        // Our own test binary must show up.
        let me = std::process::id();
        assert!(running_images().iter().any(|b| b.pid == me));
    }

    #[test]
    fn install_shape() {
        let dir = crate::testdir("shape");
        assert!(check_install_shape(&dir).is_err());
        std::fs::create_dir_all(dir.join("ROM")).unwrap();
        std::fs::write(dir.join("VTABLE.DAT"), b"v").unwrap();
        std::fs::write(dir.join("FTABLE.DAT"), b"f").unwrap();
        assert!(check_install_shape(&dir).is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }
}
