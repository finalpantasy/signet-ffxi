//! A thread-owned VM behind the persistent host; only serialized values cross DLLs.
use crate::lua::Addon;
use mlua::MultiValue;
use windower_rs::{platform::Ecosystem, runtime_options::LuaEngine, vm_protocol::Action};
#[cfg(all(windows, target_pointer_width = "32"))]
use {
    crate::vm_client::Provider,
    std::{cell::RefCell, sync::Arc},
    windower_rs::vm_protocol::{Operation, Request, Response},
};

pub enum Vm {
    Local(Addon),
    #[cfg(all(windows, target_pointer_width = "32"))]
    Foreign(Foreign),
}
#[cfg(all(windows, target_pointer_width = "32"))]
pub struct Foreign {
    provider: Arc<Provider>,
    handle: u64,
    ecosystem: Ecosystem,
    pending: RefCell<Vec<Action>>,
}
impl Vm {
    pub fn local(
        name: &str,
        runtime: &str,
        path: &str,
        source: &str,
        ecosystem: Ecosystem,
    ) -> Result<Self, String> {
        Addon::from_source_with_ecosystem(name, runtime, path, source, ecosystem)
            .map(Self::Local)
            .map_err(|e| e.to_string())
    }
    #[cfg(all(windows, target_pointer_width = "32"))]
    pub fn foreign(
        provider: Arc<Provider>,
        name: &str,
        runtime: &str,
        path: &str,
        source: &str,
        ecosystem: Ecosystem,
    ) -> Result<Self, String> {
        let response = provider.request(Request::new(Operation::Create {
            name: name.into(),
            runtime_path: runtime.into(),
            addon_path: path.into(),
            source: source.into(),
            ecosystem,
        }))?;
        if !response.ok {
            return Err(response.error.unwrap_or("Provider creation failed".into()));
        }
        let handle = response
            .handle
            .filter(|h| *h != 0)
            .ok_or("Provider returned no addon handle")?;
        let vm = Self::Foreign(Foreign {
            provider,
            handle,
            ecosystem,
            pending: RefCell::new(response.actions),
        });
        vm.scheduled_count()?;
        Ok(vm)
    }
    pub fn ecosystem(&self) -> Ecosystem {
        match self {
            Self::Local(a) => a.ecosystem(),
            #[cfg(all(windows, target_pointer_width = "32"))]
            Self::Foreign(a) => a.ecosystem,
        }
    }
    pub fn engine(&self) -> LuaEngine {
        match self {
            Self::Local(_) => LuaEngine::Lua51,
            #[cfg(all(windows, target_pointer_width = "32"))]
            Self::Foreign(_) => LuaEngine::LuaJit,
        }
    }
    pub fn emit(&self, event: &str) -> Vec<(u32, Result<(), String>)> {
        match self {
            Self::Local(a) => a
                .emit(event, MultiValue::new())
                .into_iter()
                .map(|(id, r)| (id, r.map(|_| ()).map_err(|e| e.to_string())))
                .collect(),
            #[cfg(all(windows, target_pointer_width = "32"))]
            Self::Foreign(a) => a.callbacks(Operation::Emit {
                handle: a.handle,
                event: event.into(),
                arguments: vec![],
            }),
        }
    }
    pub fn emit_frame(&self, index: u32) -> Vec<(u32, Result<(), String>)> {
        match self {
            Self::Local(a) => a
                .emit_frame(index)
                .into_iter()
                .map(|(id, r)| (id, r.map(|_| ()).map_err(|e| e.to_string())))
                .collect(),
            #[cfg(all(windows, target_pointer_width = "32"))]
            Self::Foreign(a) => a.callbacks(Operation::Frame {
                handle: a.handle,
                index,
            }),
        }
    }
    pub fn poll_at(&self, seconds: f64) -> Result<Vec<String>, String> {
        match self {
            Self::Local(a) => a.poll_at(seconds).map_err(|e| e.to_string()),
            #[cfg(all(windows, target_pointer_width = "32"))]
            Self::Foreign(a) => a
                .response(Operation::Poll {
                    handle: a.handle,
                    seconds,
                })
                .map(|r| r.errors),
        }
    }
    pub fn scheduled_count(&self) -> Result<usize, String> {
        match self {
            Self::Local(a) => a.scheduled_count().map_err(|e| e.to_string()),
            #[cfg(all(windows, target_pointer_width = "32"))]
            Self::Foreign(a) => a
                .response(Operation::Diagnostics { handle: a.handle })
                .map(|r| r.scheduled),
        }
    }
    pub fn take_actions(&self) -> Vec<Action> {
        match self {
            Self::Local(a) => a
                .take_actions()
                .into_iter()
                .map(|a| a.into_protocol())
                .collect(),
            #[cfg(all(windows, target_pointer_width = "32"))]
            Self::Foreign(a) => std::mem::take(&mut *a.pending.borrow_mut()),
        }
    }
}
#[cfg(all(windows, target_pointer_width = "32"))]
impl Foreign {
    fn response(&self, operation: Operation) -> Result<Response, String> {
        let mut response = self.provider.request(Request::new(operation))?;
        if !response.ok {
            return Err(response.error.unwrap_or("Provider operation failed".into()));
        }
        self.pending.borrow_mut().append(&mut response.actions);
        Ok(response)
    }
    fn callbacks(&self, operation: Operation) -> Vec<(u32, Result<(), String>)> {
        match self.response(operation) {
            Ok(r) => r
                .callbacks
                .into_iter()
                .map(|c| (c.id, c.error.map_or(Ok(()), Err)))
                .collect(),
            Err(e) => vec![(0, Err(e))],
        }
    }
}
#[cfg(all(windows, target_pointer_width = "32"))]
impl Drop for Foreign {
    fn drop(&mut self) {
        let _ = self.provider.request(Request::new(Operation::Destroy {
            handle: self.handle,
        }));
    }
}
