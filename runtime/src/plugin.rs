//! Windower 4.7.3.0 native plugin boundary, recovered from the shipped loader
//! and Config.dll. Only construction/metadata are implemented here.
use std::{
    ffi::{c_char, c_void},
    path::Path,
    ptr::{self, NonNull},
};
use windows_sys::Win32::{
    Foundation::{FreeLibrary, HMODULE},
    System::LibraryLoader::{
        GetProcAddress, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
        LoadLibraryExW,
    },
};

pub const INTERFACE_VERSION: u32 = 0x0407_0300;

#[repr(C)]
struct PluginObject {
    vtable: *const PluginBoundary,
}

// The explicit self argument is intentional. The first slots are stdcall C
// adapters, not thiscall methods. Config's first two adapters end in `ret 4`;
// the original Hook pushes the instance before calling slots 0, 1 and 2.
#[repr(C)]
struct PluginBoundary {
    author: unsafe extern "system" fn(*mut PluginObject) -> *const c_char,
    name: unsafe extern "system" fn(*mut PluginObject) -> *const c_char,
    initialize: unsafe extern "system" fn(*mut PluginObject, *mut c_void),
    destroy: unsafe extern "system" fn(*mut PluginObject),
}

pub struct PluginMetadata {
    pub name: String,
    pub author: String,
    pub interface_version: u32,
}

// This type is deliberately thread-confined: no Send/Sync implementation.
pub struct Plugin {
    _instance: NonNull<PluginObject>,
    module: Option<Module>,
    metadata: PluginMetadata,
}

impl Plugin {
    /// Load an explicitly selected, trusted Windower plugin. This runs its
    /// DllMain and factory, but does not call its initialization or callbacks.
    ///
    /// # Safety
    /// The module must implement Windower's 0x04070300 factory and stdcall
    /// boundary and return valid metadata strings. Use only in a dedicated
    /// metadata-probe child: stock plugins may require initialization before
    /// shutdown, so these uninitialized objects/modules are retained until
    /// process exit. Full initialization/shutdown hosting is still pending.
    /// A malicious or incompatible native module can crash the process.
    pub unsafe fn open_metadata(path: &Path) -> Result<Self, String> {
        unsafe { Self::create(Module::open(path)?) }
    }

    unsafe fn create(module: Module) -> Result<Self, String> {
        let version_address =
            unsafe { GetProcAddress(module.0, c"GetInterfaceVersion".as_ptr().cast()) }
                .ok_or("Plugin has no GetInterfaceVersion export")?;
        let get_interface: unsafe extern "system" fn() -> u32 =
            unsafe { std::mem::transmute(version_address) };
        let version = unsafe { get_interface() };
        if version != INTERFACE_VERSION {
            return Err(format!(
                "Unsupported plugin interface {version:#010x}; expected {INTERFACE_VERSION:#010x}"
            ));
        }
        let factory_address =
            unsafe { GetProcAddress(module.0, c"CreateInstance".as_ptr().cast()) }
                .ok_or("Plugin has no CreateInstance export")?;
        let factory: unsafe extern "system" fn() -> *mut PluginObject =
            unsafe { std::mem::transmute(factory_address) };
        let instance = NonNull::new(unsafe { factory() }).ok_or("Plugin factory returned null")?;
        // Once a factory succeeds, retain ownership even on metadata failure so
        // the module stays loaded until the plugin object has been destroyed.
        let mut plugin = Self {
            _instance: instance,
            module: Some(module),
            metadata: PluginMetadata {
                name: String::new(),
                author: String::new(),
                interface_version: version,
            },
        };
        let vtable = unsafe { (*instance.as_ptr()).vtable };
        if vtable.is_null() {
            // There is no safe destructor for an invalid object. Keep this
            // module loaded until process exit instead of guessing a callback.
            std::mem::forget(plugin);
            return Err("Plugin factory returned an invalid object".into());
        }
        let author = unsafe { ((*vtable).author)(instance.as_ptr()) };
        let name = unsafe { ((*vtable).name)(instance.as_ptr()) };
        // A valid trusted plugin owns these strings until destruction. Bound
        // scans so a missing terminator cannot cause an unbounded read.
        plugin.metadata.name = unsafe { read_metadata(name) }?;
        plugin.metadata.author = unsafe { read_metadata(author) }?;
        Ok(plugin)
    }

    pub fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        // Stock WinControl dereferences its host in the shutdown adapter even
        // when initialization was never called. The original loader always
        // initializes before permitting unload. Metadata-only diagnostics have
        // no host and must retain the object/module until child-process exit.
        if let Some(module) = self.module.take() {
            std::mem::forget(module);
        }
    }
}

pub(crate) struct Module(pub(crate) HMODULE);
impl Module {
    pub(crate) unsafe fn open(path: &Path) -> Result<Self, String> {
        use std::os::windows::ffi::OsStrExt;
        let absolute = path.canonicalize().map_err(|error| error.to_string())?;
        let mut name = absolute.as_os_str().encode_wide().collect::<Vec<_>>();
        if name.contains(&0) {
            return Err("NUL in plugin path".into());
        }
        name.push(0);
        let module = unsafe {
            LoadLibraryExW(
                name.as_ptr(),
                ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
            )
        };
        if module.is_null() {
            return Err(format!(
                "LoadLibraryExW: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(Self(module))
    }
}
impl Drop for Module {
    fn drop(&mut self) {
        unsafe {
            FreeLibrary(self.0);
        }
    }
}

pub(crate) unsafe fn read_metadata(pointer: *const c_char) -> Result<String, String> {
    if pointer.is_null() {
        return Err("Plugin metadata pointer is null".into());
    }
    let mut bytes = Vec::new();
    for offset in 0..4096 {
        let byte = unsafe { *pointer.add(offset) } as u8;
        if byte == 0 {
            return String::from_utf8(bytes).map_err(|_| "Plugin metadata is not UTF-8".into());
        }
        bytes.push(byte);
    }
    Err("Plugin metadata is not terminated within 4096 bytes".into())
}
