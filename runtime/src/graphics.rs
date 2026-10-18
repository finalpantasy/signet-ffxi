//! D3D8 COM proxies: system objects keep their original class layout/vtables.
//! All methods forward to the underlying object; factory/device/present/lifetime
//! are observed. Overlay rendering and game-specific routing remain pending.
use retour::RawDetour;
use std::{
    ffi::c_void,
    ptr,
    sync::{
        OnceLock,
        atomic::{AtomicU32, AtomicUsize, Ordering, fence},
    },
};
use windower_rs::runtime_options::{GraphicsBackend, RuntimeOptions};
use windows_sys::Win32::{
    Foundation::{HWND, RECT},
    System::LibraryLoader::{
        GetProcAddress, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
        LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
    },
};
type Object = *mut c_void;
type Create8 = unsafe extern "system" fn(u32) -> Object;
type Release = unsafe extern "system" fn(Object) -> u32;
type CreateDevice =
    unsafe extern "system" fn(Object, u32, u32, HWND, u32, *mut c_void, *mut Object) -> i32;
type Present =
    unsafe extern "system" fn(Object, *const RECT, *const RECT, HWND, *const c_void) -> i32;
static DETOUR: OnceLock<RawDetour> = OnceLock::new();
static ORIGINAL_CREATE: AtomicUsize = AtomicUsize::new(0);
static CREATED: AtomicU32 = AtomicU32::new(0);
static DEVICES_CREATED: AtomicU32 = AtomicU32::new(0);
static PRESENTED: AtomicU32 = AtomicU32::new(0);
static LIVE_ROOTS: AtomicU32 = AtomicU32::new(0);
static LIVE_DEVICES: AtomicU32 = AtomicU32::new(0);
static EXTERNAL_BACKEND: AtomicU32 = AtomicU32::new(0);
static EXTERNAL_CALLS: AtomicU32 = AtomicU32::new(0);

#[repr(C)]
struct Proxy {
    table: *const usize,
    references: AtomicU32,
    inner: Object,
    parent: Object,
    kind: u32,
    #[cfg(feature = "isolated-probe")]
    window: HWND,
}
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct Guid {
    a: u32,
    b: u16,
    c: u16,
    d: [u8; 8],
}
const UNKNOWN: Guid = Guid {
    a: 0,
    b: 0,
    c: 0,
    d: [0xc0, 0, 0, 0, 0, 0, 0, 0x46],
};
const ROOT: Guid = Guid {
    a: 0x1dd9e8da,
    b: 0x1c77,
    c: 0x4d40,
    d: [0xb0, 0xcf, 0x98, 0xfe, 0xfd, 0xff, 0x95, 0x12],
};
const DEVICE: Guid = Guid {
    a: 0x7385e5df,
    b: 0x8fe8,
    c: 0x41d5,
    d: [0x86, 0xb6, 0xd7, 0xb4, 0x85, 0x47, 0xb6, 0xcf],
};
const POINTER_ERROR: i32 = 0x80004003_u32 as i32;

/// Install before application threads can call Direct3DCreate8. Tests load the
/// runtime first in a dedicated host; production launch must keep its main
/// thread suspended through initialization. Detours remain until process exit.
pub unsafe fn install(backend: &GraphicsBackend) -> Result<(), String> {
    if DETOUR.get().is_some() {
        return Err("D3D8 hook already installed".into());
    }
    let name = "d3d8.dll".encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let module =
        unsafe { LoadLibraryExW(name.as_ptr(), ptr::null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32) };
    if module.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let target = unsafe { GetProcAddress(module, c"Direct3DCreate8".as_ptr().cast()) }
        .ok_or("Missing Direct3DCreate8")?;
    let external = match backend {
        GraphicsBackend::System => None,
        GraphicsBackend::External { path, .. } => {
            use std::os::windows::ffi::OsStrExt;
            RuntimeOptions {
                graphics: backend.clone(),
                ..RuntimeOptions::default()
            }
            .verify_external()?;
            let wide = path
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>();
            if wide[..wide.len() - 1].contains(&0) {
                return Err("NUL in graphics backend path".into());
            }
            let external = unsafe {
                LoadLibraryExW(
                    wide.as_ptr(),
                    ptr::null_mut(),
                    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
                )
            };
            if external.is_null() {
                return Err(format!(
                    "Graphics backend load failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let factory = unsafe { GetProcAddress(external, c"Direct3DCreate8".as_ptr().cast()) }
                .ok_or("Graphics backend has no Direct3DCreate8")?;
            if factory as usize == target as usize {
                return Err("External backend resolves to system D3D8".into());
            }
            // Retain the module until process exit, like the core hook itself.
            Some(factory as usize)
        }
    };
    let detour = unsafe { RawDetour::new(target as *const (), create8 as *const ()) }
        .map_err(|e| e.to_string())?;
    EXTERNAL_BACKEND.store(u32::from(external.is_some()), Ordering::Release);
    ORIGINAL_CREATE.store(
        external.unwrap_or(detour.trampoline() as *const () as usize),
        Ordering::Release,
    );
    DETOUR.set(detour).map_err(|_| "D3D8 installation raced")?;
    unsafe { DETOUR.get().ok_or("Missing D3D8 detour")?.enable() }.map_err(|e| e.to_string())
}

pub fn counter(which: u32) -> u32 {
    match which {
        0 => CREATED.load(Ordering::Relaxed),
        1 => DEVICES_CREATED.load(Ordering::Relaxed),
        2 => PRESENTED.load(Ordering::Relaxed),
        3 => LIVE_ROOTS.load(Ordering::Relaxed),
        4 => LIVE_DEVICES.load(Ordering::Relaxed),
        5 => EXTERNAL_BACKEND.load(Ordering::Acquire),
        6 => EXTERNAL_CALLS.load(Ordering::Relaxed),
        _ => u32::MAX,
    }
}
unsafe fn inner(object: Object) -> Object {
    unsafe { (*(object as *const Proxy)).inner }
}
unsafe fn slot(object: Object, index: usize) -> usize {
    unsafe { *(*(object as *const *const usize)).add(index) }
}

macro_rules! forward {
    ($name:ident,$slot:expr,$return:ty,($($argument:ident:$ty:ty),*))=>{
        unsafe extern "system" fn $name(object:Object,$($argument:$ty),*)->$return{
            let inner=unsafe{inner(object)};
            let address=unsafe{slot(inner,$slot)};
            let original:unsafe extern "system" fn(Object,$($ty),*)->$return=unsafe{std::mem::transmute(address)};
            unsafe{original(inner,$($argument),*)}
        }
    }
}
include!("d3d8_forwarders.rs");

unsafe extern "system" fn create8(version: u32) -> Object {
    let address = ORIGINAL_CREATE.load(Ordering::Acquire);
    if address == 0 {
        return ptr::null_mut();
    }
    let original: Create8 = unsafe { std::mem::transmute(address) };
    let object = unsafe { original(version) };
    if EXTERNAL_BACKEND.load(Ordering::Acquire) != 0 {
        EXTERNAL_CALLS.fetch_add(1, Ordering::Relaxed);
    }
    CREATED.fetch_add(1, Ordering::Relaxed);
    if object.is_null() {
        return object;
    }
    let proxy = Box::new(Proxy {
        table: root_table().as_ptr(),
        references: AtomicU32::new(1),
        inner: object,
        parent: ptr::null_mut(),
        kind: 0,
        #[cfg(feature = "isolated-probe")]
        window: ptr::null_mut(),
    });
    LIVE_ROOTS.fetch_add(1, Ordering::Relaxed);
    Box::into_raw(proxy).cast()
}

unsafe extern "system" fn query_interface(
    object: Object,
    iid: *const Guid,
    output: *mut Object,
) -> i32 {
    if iid.is_null() || output.is_null() {
        return POINTER_ERROR;
    }
    let proxy = unsafe { &*(object as *const Proxy) };
    let requested = unsafe { iid.read_unaligned() };
    if requested == UNKNOWN || requested == if proxy.kind == 0 { ROOT } else { DEVICE } {
        unsafe {
            add_ref(object);
            output.write(object);
        }
        return 0;
    }
    let original: unsafe extern "system" fn(Object, *const Guid, *mut Object) -> i32 =
        unsafe { std::mem::transmute(slot(proxy.inner, 0)) };
    unsafe { original(proxy.inner, iid, output) }
}
unsafe extern "system" fn add_ref(object: Object) -> u32 {
    unsafe {
        (*(object as *const Proxy))
            .references
            .fetch_add(1, Ordering::Relaxed)
            + 1
    }
}
unsafe extern "system" fn release(object: Object) -> u32 {
    let proxy = unsafe { &*(object as *const Proxy) };
    let previous = proxy.references.fetch_sub(1, Ordering::Release);
    if previous != 1 {
        return previous - 1;
    }
    fence(Ordering::Acquire);
    let owned = unsafe { Box::from_raw(object as *mut Proxy) };
    #[cfg(feature = "isolated-probe")]
    if owned.kind == 1 {
        unsafe {
            crate::input::unbind(owned.window);
        }
    }
    let original: Release = unsafe { std::mem::transmute(slot(owned.inner, 2)) };
    unsafe {
        original(owned.inner);
    }
    if owned.kind == 0 {
        LIVE_ROOTS.fetch_sub(1, Ordering::Relaxed);
    } else {
        LIVE_DEVICES.fetch_sub(1, Ordering::Relaxed);
    }
    if !owned.parent.is_null() {
        unsafe {
            release(owned.parent);
        }
    }
    0
}
unsafe extern "system" fn create_device(
    object: Object,
    adapter: u32,
    kind: u32,
    focus: HWND,
    flags: u32,
    parameters: *mut c_void,
    output: *mut Object,
) -> i32 {
    let inner = unsafe { inner(object) };
    let original: CreateDevice = unsafe { std::mem::transmute(slot(inner, 15)) };
    let result = unsafe { original(inner, adapter, kind, focus, flags, parameters, output) };
    if result >= 0 && !output.is_null() {
        let device = unsafe { *output };
        if !device.is_null() {
            let proxy = Box::new(Proxy {
                table: device_table().as_ptr(),
                references: AtomicU32::new(1),
                inner: device,
                parent: object,
                kind: 1,
                #[cfg(feature = "isolated-probe")]
                window: focus,
            });
            unsafe {
                add_ref(object);
                output.write(Box::into_raw(proxy).cast());
            }
            DEVICES_CREATED.fetch_add(1, Ordering::Relaxed);
            LIVE_DEVICES.fetch_add(1, Ordering::Relaxed);
            #[cfg(feature = "isolated-probe")]
            unsafe {
                crate::input::bind(focus);
            }
        }
    }
    result
}
unsafe extern "system" fn get_parent(object: Object, output: *mut Object) -> i32 {
    if output.is_null() {
        return POINTER_ERROR;
    }
    let parent = unsafe { (*(object as *const Proxy)).parent };
    unsafe {
        add_ref(parent);
        output.write(parent);
    }
    0
}
unsafe extern "system" fn present(
    object: Object,
    source: *const RECT,
    destination: *const RECT,
    window: HWND,
    dirty: *const c_void,
) -> i32 {
    let inner = unsafe { inner(object) };
    let original: Present = unsafe { std::mem::transmute(slot(inner, 15)) };
    PRESENTED.fetch_add(1, Ordering::Relaxed);
    #[cfg(feature = "client-diagnostics")]
    {
        let _ = std::panic::catch_unwind(crate::client_diagnostics::on_present);
    }
    #[cfg(feature = "isolated-probe")]
    {
        let _ = std::panic::catch_unwind(crate::frame_host::on_frame);
    }
    unsafe { original(inner, source, destination, window, dirty) }
}
