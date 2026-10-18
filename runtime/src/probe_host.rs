//! Shared infrastructure for isolated native tests. Never injects into a game.
use std::{fs, path::Path, ptr};
use windower_rs::protocol::{SETTINGS_BYTES, decode_settings};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HMODULE, INVALID_HANDLE_VALUE,
        WAIT_OBJECT_0,
    },
    System::{
        LibraryLoader::{GetProcAddress, LoadLibraryW},
        Memory::{
            CreateFileMappingW, FILE_MAP_WRITE, MapViewOfFile, PAGE_READWRITE, UnmapViewOfFile,
        },
        Threading::{CreateEventW, WaitForSingleObject},
    },
};

pub struct Host {
    mapping: HANDLE,
    event: HANDLE,
    module: HMODULE,
}
impl Host {
    pub fn start(dll: &Path, packet: &Path) -> Result<Self, String> {
        use std::os::windows::ffi::OsStrExt;
        let bytes = fs::read(packet).map_err(|e| e.to_string())?;
        let decoded = decode_settings(&bytes)?;
        if decoded.fullscreen {
            return Err("Isolated graphics tests require windowed settings".into());
        }
        let (mapping, event) = crate::mapping_names();
        let mapping = wide(&mapping);
        let event = wide(&event);
        let mapping = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                ptr::null(),
                PAGE_READWRITE,
                0,
                SETTINGS_BYTES as u32,
                mapping.as_ptr(),
            )
        };
        if mapping.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let mut host = Self {
            mapping,
            event: ptr::null_mut(),
            module: ptr::null_mut(),
        };
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            return Err("Refusing an existing test mapping".into());
        }
        host.event = unsafe { CreateEventW(ptr::null(), 0, 0, event.as_ptr()) };
        if host.event.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            return Err("Refusing an existing test event".into());
        }
        let view = unsafe { MapViewOfFile(host.mapping, FILE_MAP_WRITE, 0, 0, SETTINGS_BYTES) };
        if view.Value.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), view.Value.cast(), bytes.len());
            UnmapViewOfFile(view);
        }
        let dll = dll.canonicalize().map_err(|e| e.to_string())?;
        let dll = dll
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        host.module = unsafe { LoadLibraryW(dll.as_ptr()) };
        if host.module.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if unsafe { WaitForSingleObject(host.event, 15000) } != WAIT_OBJECT_0 {
            return Err("Rust runtime initialization did not signal readiness".into());
        }
        Ok(host)
    }
    pub fn counters(&self) -> Result<unsafe extern "system" fn(u32) -> u32, String> {
        let address =
            unsafe { GetProcAddress(self.module, c"windower_rs_graphics_counter".as_ptr().cast()) }
                .ok_or("No graphics counter export")?;
        Ok(unsafe {
            std::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                unsafe extern "system" fn(u32) -> u32,
            >(address)
        })
    }

    pub fn lua_counters(&self) -> Result<unsafe extern "system" fn(u32) -> u32, String> {
        let address =
            unsafe { GetProcAddress(self.module, c"windower_rs_lua_counter".as_ptr().cast()) }
                .ok_or("No Lua counter export")?;
        Ok(unsafe {
            std::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                unsafe extern "system" fn(u32) -> u32,
            >(address)
        })
    }

    pub fn input_counters(&self) -> Result<unsafe extern "system" fn(u32) -> u32, String> {
        let address =
            unsafe { GetProcAddress(self.module, c"windower_rs_input_counter".as_ptr().cast()) }
                .ok_or("No input counter export")?;
        Ok(unsafe {
            std::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                unsafe extern "system" fn(u32) -> u32,
            >(address)
        })
    }
    pub fn lua_shutdown(&self) -> Result<unsafe extern "system" fn() -> u32, String> {
        let address =
            unsafe { GetProcAddress(self.module, c"windower_rs_lua_shutdown".as_ptr().cast()) }
                .ok_or("No Lua shutdown export")?;
        Ok(unsafe {
            std::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                unsafe extern "system" fn() -> u32,
            >(address)
        })
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        unsafe {
            if !self.event.is_null() {
                CloseHandle(self.event);
            }
            if !self.mapping.is_null() {
                CloseHandle(self.mapping);
            }
        }
    }
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
