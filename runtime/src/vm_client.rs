//! Scoped client for the VM-provider C ABI. Response sizing reads an existing
//! response token; it never re-executes the original Lua request.
use crate::plugin::Module;
use std::{path::Path, ptr};
use windower_rs::vm_protocol::*;
use windows_sys::Win32::System::LibraryLoader::GetProcAddress;

pub struct Provider {
    module: Option<Module>,
    api: ProviderApiV1,
}
// HMODULE is a process-wide loader reference, not a thread-affine plugin
// object. ABI v1 providers serialize their registry and individual Lua states.
// Arc/shared borrows prevent Drop while a safe request is executing.
unsafe impl Send for Provider {}
unsafe impl Sync for Provider {}

impl Provider {
    /// Load an explicit trusted, same-architecture Rust VM-provider DLL.
    ///
    /// # Safety
    /// Its native code and thread-safe C API must be trusted. Supplied Lua source has
    /// io/os/debug/FFI access and is not sandboxed. Do not unload the provider
    /// while another thread is executing one of its entry points.
    pub unsafe fn open(path: &Path) -> Result<Self, String> {
        let module = unsafe { Module::open(path)? };
        let getter = unsafe { GetProcAddress(module.0, c"windower_vm_api_v1".as_ptr().cast()) }
            .ok_or("Missing VM provider API export")?;
        let getter: unsafe extern "system" fn() -> *const ProviderApiV1 =
            unsafe { std::mem::transmute(getter) };
        let address = unsafe { getter() };
        if address.is_null() {
            return Err("VM provider returned a null API".into());
        }
        let header = unsafe { ptr::read_unaligned(address.cast::<[u32; 4]>()) };
        if header[0] != std::mem::size_of::<ProviderApiV1>() as u32
            || header[1] != ABI_VERSION
            || header[2] != usize::BITS
            || header[3] != ENGINE_LUAJIT
        {
            return Err("Incompatible VM provider header".into());
        }
        let api = unsafe { ptr::read_unaligned(address) };
        api.validate()?;
        Ok(Self {
            module: Some(module),
            api,
        })
    }

    pub fn request(&self, request: Request) -> Result<Response, String> {
        let bytes = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
        if bytes.is_empty() || bytes.len() > MAX_REQUEST_BYTES {
            return Err("VM request exceeds size limit".into());
        }
        let mut token = 0;
        let status = unsafe {
            (self.api.request.ok_or("Missing VM request entry")?)(
                bytes.as_ptr(),
                bytes.len() as u32,
                &mut token,
            )
        };
        if status != OK || token == 0 {
            return Err(format!("VM request failed with status {status}"));
        }
        struct ResponseGuard<'a>(&'a ProviderApiV1, u64);
        impl Drop for ResponseGuard<'_> {
            fn drop(&mut self) {
                if let Some(release) = self.0.release_response {
                    unsafe {
                        release(self.1);
                    }
                }
            }
        }
        let guard = ResponseGuard(&self.api, token);
        let read = self.api.read_response.ok_or("Missing VM response reader")?;
        let mut needed = 0;
        let status = unsafe { read(token, ptr::null_mut(), 0, &mut needed) };
        if status != BUFFER_TOO_SMALL || needed == 0 || needed as usize > MAX_RESPONSE_BYTES {
            return Err(format!(
                "Invalid VM response size/status: {needed}/{status}"
            ));
        }
        let mut bytes = vec![0; needed as usize];
        let mut copied = 0;
        let status = unsafe { read(token, bytes.as_mut_ptr(), bytes.len() as u32, &mut copied) };
        if status != OK || copied != needed {
            return Err("VM response changed during read".into());
        }
        let response = serde_json::from_slice(&bytes).map_err(|error| error.to_string());
        drop(guard);
        response
    }

    pub fn resource_counts(&self) -> Result<(u32, u32), String> {
        let (mut addons, mut responses) = (0, 0);
        let status = unsafe {
            (self.api.counts.ok_or("Missing VM counts entry")?)(&mut addons, &mut responses)
        };
        if status != OK {
            return Err(format!("VM resource query failed: {status}"));
        }
        Ok((addons, responses))
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        // A caller that forgets Destroy must not leave live Lua/JIT objects
        // referring to unmapped code. Retain this module on an incomplete
        // cleanup; correct clients destroy all addons and response tokens.
        if self.resource_counts() != Ok((0, 0))
            && let Some(module) = self.module.take()
        {
            std::mem::forget(module);
        }
    }
}
