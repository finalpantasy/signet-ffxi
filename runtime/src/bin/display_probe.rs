//! Registry reads only, in the newly created fixture with display hooks enabled.
#[cfg(all(windows, target_pointer_width = "32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{ffi::CString, io::Write, os::windows::ffi::OsStrExt, path::Path, ptr};
    use windows_sys::Win32::{
        Foundation::{ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS},
        System::{
            LibraryLoader::{GetModuleHandleW, GetProcAddress},
            Registry::*,
        },
    };
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.len() != 3 || args[0] != "--injected" {
        return Err("Use the owned suspended fixture launcher".into());
    }
    let key_path = c"SOFTWARE\\PlayOnlineUS\\SquareEnix\\FinalFantasyXI";
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let mut key = ptr::null_mut();
    if unsafe {
        RegOpenKeyExA(
            HKEY_LOCAL_MACHINE,
            key_path.as_ptr().cast(),
            0,
            KEY_READ | KEY_WOW64_32KEY,
            &mut key,
        )
    } != 0
    {
        return Err("Installed US display key not found".into());
    }
    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }
    let key = Key(key);
    let expected = [
        ("0001", 1280u32),
        ("0002", 720),
        ("0003", 1280),
        ("0004", 720),
        ("0034", 1),
        ("0037", 640),
        ("0038", 360),
    ];
    for (name, value) in expected {
        let a = CString::new(name)?;
        let w = wide(name);
        let mut kind = 0;
        let mut size = 4;
        let mut observed = 0u32;
        if unsafe {
            RegQueryValueExA(
                key.0,
                a.as_ptr().cast(),
                ptr::null(),
                &mut kind,
                (&mut observed as *mut u32).cast(),
                &mut size,
            )
        } != 0
            || observed != value
            || size != 4
            || kind != REG_DWORD
        {
            return Err(format!("ANSI display query failed: {name}").into());
        }
        size = 4;
        observed = 0;
        if unsafe {
            RegQueryValueExW(
                key.0,
                w.as_ptr(),
                ptr::null(),
                &mut kind,
                (&mut observed as *mut u32).cast(),
                &mut size,
            )
        } != 0
            || observed != value
        {
            return Err("Wide display query failed".into());
        }
        size = 4;
        observed = 0;
        if unsafe {
            RegGetValueA(
                HKEY_LOCAL_MACHINE,
                key_path.as_ptr().cast(),
                a.as_ptr().cast(),
                RRF_RT_REG_DWORD,
                &mut kind,
                (&mut observed as *mut u32).cast(),
                &mut size,
            )
        } != 0
            || observed != value
        {
            return Err("ANSI relative display query failed".into());
        }
        size = 4;
        observed = 0;
        if unsafe {
            RegGetValueW(
                key.0,
                ptr::null(),
                w.as_ptr(),
                RRF_RT_REG_DWORD,
                &mut kind,
                (&mut observed as *mut u32).cast(),
                &mut size,
            )
        } != 0
            || observed != value
        {
            return Err("Wide relative display query failed".into());
        }
    }
    let mut bytes = [0xaau8; 2];
    let mut size = 2;
    if unsafe {
        RegQueryValueExA(
            key.0,
            c"0001".as_ptr().cast(),
            ptr::null(),
            ptr::null_mut(),
            bytes.as_mut_ptr(),
            &mut size,
        )
    } != ERROR_MORE_DATA
        || size != 4
        || bytes != [0xaa; 2]
    {
        return Err("Short query buffer was overwritten".into());
    }
    size = 2;
    if unsafe {
        RegGetValueA(
            key.0,
            ptr::null(),
            c"0001".as_ptr().cast(),
            RRF_RT_REG_DWORD | RRF_ZEROONFAILURE,
            ptr::null_mut(),
            bytes.as_mut_ptr().cast(),
            &mut size,
        )
    } != ERROR_MORE_DATA
        || size != 4
        || bytes != [0; 2]
    {
        return Err("RegGetValue zero-on-failure contract failed".into());
    }
    let mut found_a = 0;
    let mut found_w = 0;
    for index in 0..512 {
        let mut a = [0u8; 512];
        let mut n = 512;
        let mut data = [0u8; 4096];
        let mut size = 4096;
        let mut kind = 0;
        let status = unsafe {
            RegEnumValueA(
                key.0,
                index,
                a.as_mut_ptr(),
                &mut n,
                ptr::null(),
                &mut kind,
                data.as_mut_ptr(),
                &mut size,
            )
        };
        if status == ERROR_NO_MORE_ITEMS {
            break;
        }
        if status != 0 {
            return Err("ANSI display enumeration failed".into());
        }
        if let Some((_, value)) = expected
            .iter()
            .find(|(name, _)| name.as_bytes() == &a[..n as usize])
        {
            if kind != REG_DWORD || size != 4 || u32::from_le_bytes(data[..4].try_into()?) != *value
            {
                return Err("ANSI enumeration did not use virtual values".into());
            }
            found_a += 1;
        }
        let mut w = [0u16; 512];
        n = 512;
        size = 4096;
        if unsafe {
            RegEnumValueW(
                key.0,
                index,
                w.as_mut_ptr(),
                &mut n,
                ptr::null(),
                &mut kind,
                data.as_mut_ptr(),
                &mut size,
            )
        } != 0
        {
            return Err("Wide display enumeration failed".into());
        }
        let name = String::from_utf16(&w[..n as usize])?;
        if let Some((_, value)) = expected.iter().find(|(s, _)| *s == name) {
            if kind != REG_DWORD || size != 4 || u32::from_le_bytes(data[..4].try_into()?) != *value
            {
                return Err("Wide enumeration did not use virtual values".into());
            }
            found_w += 1;
        }
    }
    if found_a != 7 || found_w != 7 {
        return Err("Not all display values were enumerated".into());
    }
    let mut install = ptr::null_mut();
    if unsafe {
        RegOpenKeyExA(
            HKEY_LOCAL_MACHINE,
            c"SOFTWARE\\PlayOnlineUS\\InstallFolder".as_ptr().cast(),
            0,
            KEY_READ | KEY_WOW64_32KEY,
            &mut install,
        )
    } != 0
    {
        return Err("Missing non-display reference key".into());
    }
    let install = Key(install);
    let mut kind = 0;
    let mut size = 0;
    if unsafe {
        RegQueryValueExA(
            install.0,
            c"0001".as_ptr().cast(),
            ptr::null(),
            &mut kind,
            ptr::null_mut(),
            &mut size,
        )
    } != 0
        || kind != REG_SZ
        || size <= 4
    {
        return Err("Unrelated same-name registry value was overridden".into());
    }
    let module_name = Path::new(&args[1])
        .file_name()
        .ok_or("Missing module name")?
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let module = unsafe { GetModuleHandleW(module_name.as_ptr()) };
    let counter = unsafe {
        GetProcAddress(
            module,
            c"windower_rs_virtual_display_overrides".as_ptr().cast(),
        )
    }
    .ok_or("Missing virtual display counter")?;
    let counter: unsafe extern "system" fn() -> u32 = unsafe { std::mem::transmute(counter) };
    let overrides = unsafe { counter() };
    if overrides < 42 {
        return Err("Display read interception was incomplete".into());
    }
    let report = serde_json::json!({"stage":"process_local_display_reads","query_a":true,"query_w":true,"get_a":true,"get_w":true,"enum_a":true,"enum_w":true,"short_buffer_preserved":true,"get_zero_on_failure":true,"non_display_key_forwarded":true,"overrides":overrides,"registry_writes":0,"game_started":false});
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&args[2])?;
    file.write_all(&serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}
#[cfg(not(all(windows, target_pointer_width = "32")))]
fn main() {
    eprintln!("Build for i686-pc-windows-msvc");
    std::process::exit(1);
}
