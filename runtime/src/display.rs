//! Initial process-local display registry reads. Install before the owned
//! child's application thread resumes. Never writes a registry value.
use retour::RawDetour;
use std::{
    ffi::{c_char, c_void},
    ptr,
    sync::{
        OnceLock,
        atomic::{AtomicU32, Ordering},
    },
};
use windower_rs::{display::DisplayValues, protocol::DecodedSettings};
use windows_sys::Win32::{
    Foundation::{ERROR_INVALID_PARAMETER, ERROR_MORE_DATA, ERROR_SUCCESS},
    System::{
        Diagnostics::Debug::ReadProcessMemory,
        LibraryLoader::{
            GetModuleHandleW, GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
        },
        Registry::{HKEY, HKEY_LOCAL_MACHINE, REG_DWORD},
        Threading::GetCurrentProcess,
    },
};
type QueryA =
    unsafe extern "system" fn(HKEY, *const u8, *const u32, *mut u32, *mut u8, *mut u32) -> u32;
type QueryW =
    unsafe extern "system" fn(HKEY, *const u16, *const u32, *mut u32, *mut u8, *mut u32) -> u32;
type GetA = unsafe extern "system" fn(
    HKEY,
    *const u8,
    *const u8,
    u32,
    *mut u32,
    *mut c_void,
    *mut u32,
) -> u32;
type GetW = unsafe extern "system" fn(
    HKEY,
    *const u16,
    *const u16,
    u32,
    *mut u32,
    *mut c_void,
    *mut u32,
) -> u32;
type EnumA = unsafe extern "system" fn(
    HKEY,
    u32,
    *mut u8,
    *mut u32,
    *const u32,
    *mut u32,
    *mut u8,
    *mut u32,
) -> u32;
type EnumW = unsafe extern "system" fn(
    HKEY,
    u32,
    *mut u16,
    *mut u32,
    *const u32,
    *mut u32,
    *mut u8,
    *mut u32,
) -> u32;
type NtQueryKey = unsafe extern "system" fn(HKEY, u32, *mut c_void, u32, *mut u32) -> i32;
struct Hooks {
    originals: [usize; 6],
    detours: Vec<RawDetour>,
    values: DisplayValues,
    keys: Vec<Vec<u16>>,
    query: NtQueryKey,
}
static HOOKS: OnceLock<Hooks> = OnceLock::new();
static OVERRIDES: AtomicU32 = AtomicU32::new(0);
const KEYS: [&str; 6] = [
    "\\REGISTRY\\MACHINE\\SOFTWARE\\Wow6432Node\\PlayOnlineUS\\SquareEnix\\FinalFantasyXI",
    "\\REGISTRY\\MACHINE\\SOFTWARE\\Wow6432Node\\PlayOnline\\Square\\FinalFantasyXI",
    "\\REGISTRY\\MACHINE\\SOFTWARE\\Wow6432Node\\PlayOnlineEU\\SquareEnix\\FinalFantasyXI",
    "\\REGISTRY\\MACHINE\\SOFTWARE\\PlayOnlineUS\\SquareEnix\\FinalFantasyXI",
    "\\REGISTRY\\MACHINE\\SOFTWARE\\PlayOnline\\Square\\FinalFantasyXI",
    "\\REGISTRY\\MACHINE\\SOFTWARE\\PlayOnlineEU\\SquareEnix\\FinalFantasyXI",
];
/// # Safety
/// The newly created application must still be suspended, with no game code
/// running. System API detours/core remain mapped for the process lifetime.
pub unsafe fn install(settings: &DecodedSettings) -> Result<(), String> {
    if HOOKS.get().is_some() {
        return Err("Display hooks already installed".into());
    }
    let values = DisplayValues::from_settings(settings)?;
    let library = "advapi32.dll"
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let module = unsafe {
        LoadLibraryExW(
            library.as_ptr(),
            ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    if module.is_null() {
        return Err("Could not load system registry APIs".into());
    }
    let ntdll = "ntdll.dll"
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let query = unsafe {
        GetProcAddress(
            GetModuleHandleW(ntdll.as_ptr()),
            c"NtQueryKey".as_ptr().cast(),
        )
    }
    .ok_or("Missing NtQueryKey")?;
    let names = [
        c"RegQueryValueExA",
        c"RegQueryValueExW",
        c"RegGetValueA",
        c"RegGetValueW",
        c"RegEnumValueA",
        c"RegEnumValueW",
    ];
    let callbacks = [
        query_a as *const (),
        query_w as *const (),
        get_a as *const (),
        get_w as *const (),
        enum_a as *const (),
        enum_w as *const (),
    ];
    let mut detours = Vec::new();
    let mut originals = [0; 6];
    for (i, name) in names.iter().enumerate() {
        let target = unsafe { GetProcAddress(module, name.as_ptr().cast()) }
            .ok_or("Missing registry API")?;
        let detour = unsafe { RawDetour::new(target as *const (), callbacks[i]) }
            .map_err(|e| e.to_string())?;
        originals[i] = detour.trampoline() as *const () as usize;
        detours.push(detour);
    }
    HOOKS
        .set(Hooks {
            originals,
            detours,
            values,
            keys: KEYS.iter().map(|s| s.encode_utf16().collect()).collect(),
            query: unsafe {
                std::mem::transmute::<unsafe extern "system" fn() -> isize, NtQueryKey>(query)
            },
        })
        .map_err(|_| "Display hook installation raced")?;
    for detour in &HOOKS.get().ok_or("Missing display hooks")?.detours {
        unsafe { detour.enable() }.map_err(|e| e.to_string())?;
    }
    Ok(())
}
pub fn overrides() -> u32 {
    OVERRIDES.load(Ordering::Relaxed)
}
fn target_key(key: HKEY, sub: &[u16]) -> bool {
    let Some(hooks) = HOOKS.get() else {
        return false;
    };
    #[repr(C)]
    struct Name {
        length: u32,
        text: [u16; 1024],
    }
    let mut buffer = Name {
        length: 0,
        text: [0; 1024],
    };
    let mut used = 0;
    let root = if key == HKEY_LOCAL_MACHINE {
        const MACHINE: [u16; 17] = [
            92, 82, 69, 71, 73, 83, 84, 82, 89, 92, 77, 65, 67, 72, 73, 78, 69,
        ];
        &MACHINE[..]
    } else {
        if unsafe {
            (hooks.query)(
                key,
                3,
                (&mut buffer as *mut Name).cast(),
                std::mem::size_of::<Name>() as u32,
                &mut used,
            )
        } != 0
            || !buffer.length.is_multiple_of(2)
            || buffer.length as usize > buffer.text.len() * 2
        {
            return false;
        }
        &buffer.text[..buffer.length as usize / 2]
    };
    let length = root.len() + if sub.is_empty() { 0 } else { 1 + sub.len() };
    hooks.keys.iter().any(|expected| {
        if expected.len() != length {
            return false;
        }
        let separator = if sub.is_empty() { None } else { Some(92u16) };
        root.iter()
            .copied()
            .chain(separator)
            .chain(sub.iter().copied())
            .zip(expected.iter().copied())
            .all(|(a, b)| fold(a) == fold(b))
    })
}
fn fold(c: u16) -> u16 {
    if (65..=90).contains(&c) { c + 32 } else { c }
}
fn name_a(name: *const u8) -> Option<[u8; 4]> {
    let mut bytes = [0u8; 5];
    let mut read = 0;
    if name.is_null()
        || unsafe {
            ReadProcessMemory(
                GetCurrentProcess(),
                name.cast(),
                bytes.as_mut_ptr().cast(),
                5,
                &mut read,
            )
        } == 0
        || read != 5
        || bytes[4] != 0
    {
        return None;
    }
    bytes[..4].try_into().ok()
}
fn name_w(name: *const u16) -> Option<[u8; 4]> {
    let mut bytes = [0u16; 5];
    let mut read = 0;
    if name.is_null()
        || unsafe {
            ReadProcessMemory(
                GetCurrentProcess(),
                name.cast(),
                bytes.as_mut_ptr().cast(),
                10,
                &mut read,
            )
        } == 0
        || read != 10
        || bytes[4] != 0
        || bytes[..4].iter().any(|v| *v > 127)
    {
        return None;
    }
    Some([
        bytes[0] as u8,
        bytes[1] as u8,
        bytes[2] as u8,
        bytes[3] as u8,
    ])
}
unsafe fn write_dword(value: u32, kind: *mut u32, data: *mut u8, size: *mut u32) -> u32 {
    if !data.is_null() && size.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let capacity = if size.is_null() { 0 } else { unsafe { *size } };
    if !kind.is_null() {
        unsafe {
            *kind = REG_DWORD;
        }
    }
    if !size.is_null() {
        unsafe {
            *size = 4;
        }
    }
    if !data.is_null() {
        if capacity < 4 {
            return ERROR_MORE_DATA;
        }
        unsafe {
            ptr::copy_nonoverlapping(value.to_le_bytes().as_ptr(), data, 4);
        }
    }
    OVERRIDES.fetch_add(1, Ordering::Relaxed);
    ERROR_SUCCESS
}
fn setting(key: HKEY, sub: &[u16], name: Option<[u8; 4]>) -> Option<u32> {
    let hooks = HOOKS.get()?;
    let value = hooks.values.setting(&name?)?;
    if target_key(key, sub) {
        Some(value)
    } else {
        None
    }
}
fn supported_flags(flags: u32) -> bool {
    flags & !0x3003ffff == 0
        && flags & 0x30000 != 0x30000
        && (flags & 0xffff == 0 || flags & 0x10 != 0)
}
unsafe fn get_dword(
    value: u32,
    flags: u32,
    kind: *mut u32,
    data: *mut c_void,
    size: *mut u32,
) -> u32 {
    let capacity = if size.is_null() { 0 } else { unsafe { *size } };
    let result = unsafe { write_dword(value, kind, data.cast(), size) };
    if result != 0 && flags & 0x20000000 != 0 && !data.is_null() {
        unsafe {
            ptr::write_bytes(data.cast::<u8>(), 0, capacity as usize);
        }
    }
    result
}
unsafe extern "system" fn query_a(
    key: HKEY,
    name: *const u8,
    reserved: *const u32,
    kind: *mut u32,
    data: *mut u8,
    size: *mut u32,
) -> u32 {
    if reserved.is_null()
        && let Some(value) = setting(key, &[], name_a(name))
    {
        return unsafe { write_dword(value, kind, data, size) };
    }
    let original: QueryA =
        unsafe { std::mem::transmute(HOOKS.get().expect("Installed display state").originals[0]) };
    unsafe { original(key, name, reserved, kind, data, size) }
}
unsafe extern "system" fn query_w(
    key: HKEY,
    name: *const u16,
    reserved: *const u32,
    kind: *mut u32,
    data: *mut u8,
    size: *mut u32,
) -> u32 {
    if reserved.is_null()
        && let Some(value) = setting(key, &[], name_w(name))
    {
        return unsafe { write_dword(value, kind, data, size) };
    }
    let original: QueryW =
        unsafe { std::mem::transmute(HOOKS.get().expect("Installed display state").originals[1]) };
    unsafe { original(key, name, reserved, kind, data, size) }
}
unsafe fn sub_a(sub: *const u8) -> Vec<u16> {
    if sub.is_null() {
        Vec::new()
    } else {
        unsafe { std::ffi::CStr::from_ptr(sub as *const c_char) }
            .to_bytes()
            .iter()
            .map(|b| *b as u16)
            .collect()
    }
}
unsafe fn sub_w(sub: *const u16) -> Vec<u16> {
    if sub.is_null() {
        return Vec::new();
    }
    let mut length = 0;
    while length < 1024 && unsafe { *sub.add(length) } != 0 {
        length += 1;
    }
    unsafe { std::slice::from_raw_parts(sub, length) }.to_vec()
}
unsafe extern "system" fn get_a(
    key: HKEY,
    sub: *const u8,
    name: *const u8,
    flags: u32,
    kind: *mut u32,
    data: *mut c_void,
    size: *mut u32,
) -> u32 {
    if supported_flags(flags)
        && let Some(value) = setting(key, &unsafe { sub_a(sub) }, name_a(name))
    {
        return unsafe { get_dword(value, flags, kind, data, size) };
    }
    let original: GetA =
        unsafe { std::mem::transmute(HOOKS.get().expect("Installed display state").originals[2]) };
    unsafe { original(key, sub, name, flags, kind, data, size) }
}
unsafe extern "system" fn get_w(
    key: HKEY,
    sub: *const u16,
    name: *const u16,
    flags: u32,
    kind: *mut u32,
    data: *mut c_void,
    size: *mut u32,
) -> u32 {
    if supported_flags(flags)
        && let Some(value) = setting(key, &unsafe { sub_w(sub) }, name_w(name))
    {
        return unsafe { get_dword(value, flags, kind, data, size) };
    }
    let original: GetW =
        unsafe { std::mem::transmute(HOOKS.get().expect("Installed display state").originals[3]) };
    unsafe { original(key, sub, name, flags, kind, data, size) }
}
unsafe extern "system" fn enum_a(
    key: HKEY,
    index: u32,
    name: *mut u8,
    length: *mut u32,
    reserved: *const u32,
    kind: *mut u32,
    data: *mut u8,
    size: *mut u32,
) -> u32 {
    let original: EnumA =
        unsafe { std::mem::transmute(HOOKS.get().expect("Installed display state").originals[4]) };
    let capacity = if size.is_null() { 0 } else { unsafe { *size } };
    let result = unsafe { original(key, index, name, length, reserved, kind, data, size) };
    if result == 0
        && reserved.is_null()
        && let Some(value) = setting(key, &[], name_a(name))
    {
        if !size.is_null() {
            unsafe {
                *size = capacity;
            }
        }
        return unsafe { write_dword(value, kind, data, size) };
    }
    result
}
unsafe extern "system" fn enum_w(
    key: HKEY,
    index: u32,
    name: *mut u16,
    length: *mut u32,
    reserved: *const u32,
    kind: *mut u32,
    data: *mut u8,
    size: *mut u32,
) -> u32 {
    let original: EnumW =
        unsafe { std::mem::transmute(HOOKS.get().expect("Installed display state").originals[5]) };
    let capacity = if size.is_null() { 0 } else { unsafe { *size } };
    let result = unsafe { original(key, index, name, length, reserved, kind, data, size) };
    if result == 0
        && reserved.is_null()
        && let Some(value) = setting(key, &[], name_w(name))
    {
        if !size.is_null() {
            unsafe {
                *size = capacity;
            }
        }
        return unsafe { write_dword(value, kind, data, size) };
    }
    result
}
