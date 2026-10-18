//! Ashita 4.30 factory/metadata boundary. Public SDK functions are thiscall
//! virtual methods; factory/version exports are stdcall. These are different
//! from Windower's explicit-self stdcall vtable. Full lifecycle is pending.
use crate::plugin::{Module, read_metadata};
use std::{ffi::c_char, path::Path, ptr::NonNull};
use windows_sys::Win32::System::LibraryLoader::GetProcAddress;

pub const INTERFACE_VERSION: f64 = 4.30;

#[repr(C)]
struct Object {
    vtable: *const MetadataBoundary,
}

// Prefix of IPluginBase in the pinned official Ashita SDK. The MSVC x86
// thiscall ABI places self in ECX and returns double through the x87 stack.
#[repr(C)]
struct MetadataBoundary {
    name: unsafe extern "thiscall" fn(*mut Object) -> *const c_char,
    author: unsafe extern "thiscall" fn(*mut Object) -> *const c_char,
    description: unsafe extern "thiscall" fn(*mut Object) -> *const c_char,
    link: unsafe extern "thiscall" fn(*mut Object) -> *const c_char,
    version: unsafe extern "thiscall" fn(*mut Object) -> f64,
    interface_version: unsafe extern "thiscall" fn(*mut Object) -> f64,
    priority: unsafe extern "thiscall" fn(*mut Object) -> i32,
    flags: unsafe extern "thiscall" fn(*mut Object) -> u32,
}

pub struct Metadata {
    pub name: String,
    pub author: String,
    pub description: String,
    pub link: String,
    pub version: f64,
    pub interface_version: f64,
    pub priority: i32,
    pub flags: u32,
}

// Thread-confined, and deliberately not a production plugin lifecycle type.
pub struct Plugin {
    _instance: NonNull<Object>,
    module: Option<Module>,
    metadata: Metadata,
}

impl Plugin {
    /// Construct and inspect a trusted native Ashita plugin in a disposable
    /// child. Does not initialize against an IAshitaCore or invoke callbacks.
    ///
    /// # Safety
    /// The chosen DLL executes native code and must implement the pinned
    /// Ashita 4.30 SDK and valid metadata strings. A bad DLL can crash the
    /// process. Only use in the dedicated probe. The uninitialized instance
    /// and module are retained until process exit because arbitrary plugin
    /// destruction before Initialize has not been established as safe.
    pub unsafe fn open_metadata(path: &Path) -> Result<Self, String> {
        let module = unsafe { Module::open(path)? };
        let version =
            unsafe { GetProcAddress(module.0, c"expGetInterfaceVersion".as_ptr().cast()) }
                .ok_or("Plugin has no expGetInterfaceVersion export")?;
        let get_version: unsafe extern "system" fn() -> f64 =
            unsafe { std::mem::transmute(version) };
        let version = unsafe { get_version() };
        if version != INTERFACE_VERSION {
            return Err(format!(
                "Unsupported Ashita interface {version}; expected {INTERFACE_VERSION}"
            ));
        }
        let factory = unsafe { GetProcAddress(module.0, c"expCreatePlugin".as_ptr().cast()) }
            .ok_or("Plugin has no expCreatePlugin export")?;
        let factory: unsafe extern "system" fn(*const c_char) -> *mut Object =
            unsafe { std::mem::transmute(factory) };
        let instance =
            NonNull::new(unsafe { factory(c"".as_ptr()) }).ok_or("Ashita factory returned null")?;
        let mut plugin = Self {
            _instance: instance,
            module: Some(module),
            metadata: Metadata {
                name: String::new(),
                author: String::new(),
                description: String::new(),
                link: String::new(),
                version: 0.0,
                interface_version: version,
                priority: 0,
                flags: 0,
            },
        };
        let vtable = unsafe { (*instance.as_ptr()).vtable };
        if vtable.is_null() {
            return Err("Ashita factory returned a null vtable".into());
        }
        let object = instance.as_ptr();
        let table = unsafe { &*vtable };
        plugin.metadata.name = unsafe { read_metadata((table.name)(object))? };
        plugin.metadata.author = unsafe { read_metadata((table.author)(object))? };
        plugin.metadata.description = unsafe { read_metadata((table.description)(object))? };
        plugin.metadata.link = unsafe { read_metadata((table.link)(object))? };
        plugin.metadata.version = unsafe { (table.version)(object) };
        plugin.metadata.interface_version = unsafe { (table.interface_version)(object) };
        plugin.metadata.priority = unsafe { (table.priority)(object) };
        plugin.metadata.flags = unsafe { (table.flags)(object) };
        if plugin.metadata.interface_version != INTERFACE_VERSION
            || !plugin.metadata.version.is_finite()
        {
            return Err("Ashita metadata version disagrees with the selected SDK".into());
        }
        Ok(plugin)
    }

    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        if let Some(module) = self.module.take() {
            std::mem::forget(module);
        }
    }
}
