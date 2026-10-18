//! Render/input event integration for isolated verification builds. Host locks
//! cover bookkeeping only; never hold them while executing arbitrary Lua.
use crate::lua::{Addon, HostAction};
use crate::vm_client::Provider;
use mlua::{MultiValue, Value};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::Instant,
};
use windower_rs::{
    platform::Ecosystem,
    runtime_options::{AddonSpec, LuaEngine},
    vm_protocol::{self as vm, Operation, Request, Response},
};
enum HostedAddon {
    Local(Addon),
    Foreign {
        provider: Arc<Provider>,
        handle: u64,
        ecosystem: Ecosystem,
        epoch: Instant,
    },
}
impl HostedAddon {
    fn response(&self, operation: Operation) -> Result<Response, String> {
        let Self::Foreign { provider, .. } = self else {
            return Err("Expected foreign VM".into());
        };
        let response = provider.request(Request::new(operation))?;
        if !response.ok {
            return Err(response.error.unwrap_or("Unknown provider error".into()));
        }
        Ok(response)
    }
    fn event(&self, event: &str) {
        match self {
            Self::Local(addon) => {
                record(addon.emit(event, MultiValue::new()), addon.take_actions())
            }
            Self::Foreign { handle, .. } => record_foreign(self.response(Operation::Emit {
                handle: *handle,
                event: event.into(),
                arguments: Vec::new(),
            })),
        }
    }
    fn frame(&self, index: u32) {
        match self {
            Self::Local(addon) => {
                match addon.poll() {
                    Ok(errors) => {
                        for message in errors {
                            error(message);
                        }
                    }
                    Err(message) => error(message.to_string()),
                }
                record(addon.emit_frame(index), addon.take_actions());
            }
            Self::Foreign { handle, epoch, .. } => {
                record_foreign(self.response(Operation::Poll {
                    handle: *handle,
                    seconds: epoch.elapsed().as_secs_f64(),
                }));
                record_foreign(self.response(Operation::Frame {
                    handle: *handle,
                    index,
                }));
            }
        }
    }
    fn input(&self, name: &str, args: MultiValue, blocked_at: usize, blocked: bool) -> bool {
        match self {
            Self::Local(addon) => {
                if addon.ecosystem() == Ecosystem::Ashita4 {
                    return blocked;
                }
                let (next, outcomes) = addon.emit_blocking(name, args, blocked_at, blocked);
                record(outcomes, addon.take_actions());
                next
            }
            Self::Foreign {
                handle, ecosystem, ..
            } => {
                // Ashita input requires its table-event adapter, still pending.
                if *ecosystem == Ecosystem::Ashita4 {
                    return blocked;
                }
                let arguments = args
                    .into_iter()
                    .map(|value| match value {
                        Value::Boolean(value) => vm::Value::Boolean(value),
                        Value::Integer(value) => vm::Value::Number(value as f64),
                        _ => vm::Value::Nil,
                    })
                    .collect();
                let response = self.response(Operation::EmitBlocking {
                    handle: *handle,
                    event: name.into(),
                    arguments,
                    blocked_at,
                    blocked,
                });
                let next = response
                    .as_ref()
                    .map(|value| value.blocked)
                    .unwrap_or(blocked);
                record_foreign(response);
                next
            }
        }
    }
}
impl Drop for HostedAddon {
    fn drop(&mut self) {
        if let Self::Foreign {
            provider, handle, ..
        } = self
        {
            // Provider retains its module if cleanup cannot complete.
            let _ = provider.request(Request::new(Operation::Destroy { handle: *handle }));
        }
    }
}
#[derive(Default)]
struct Host {
    addons: Vec<LoadedAddon>,
    frames: u32,
    callbacks: u32,
    queue: Vec<HostAction>,
    errors: Vec<String>,
    loading: bool,
    loaded: bool,
    dispatches: usize,
    stopped: bool,
    stopping: bool,
    reloads: u32,
}
struct LoadedAddon {
    spec: AddonSpec,
    addon: Arc<HostedAddon>,
}
struct ReloadPlan {
    index: usize,
    spec: AddonSpec,
    provider: Option<Arc<Provider>>,
}
impl Host {
    fn reserve_reload(&mut self, name: &str) -> Result<ReloadPlan, String> {
        if self.loading || self.stopping || self.dispatches != 0 {
            return Err("Addon host is busy; retry after dispatch completes".into());
        }
        if !self.loaded || self.stopped {
            return Err("Addon host is not running".into());
        }
        let index = self
            .addons
            .iter()
            .position(|entry| entry.spec.name.eq_ignore_ascii_case(name))
            .ok_or("Addon is not loaded")?;
        let entry = &self.addons[index];
        let provider = match entry.addon.as_ref() {
            HostedAddon::Foreign { provider, .. } => Some(provider.clone()),
            _ => None,
        };
        let plan = ReloadPlan {
            index,
            spec: entry.spec.clone(),
            provider,
        };
        self.loading = true;
        Ok(plan)
    }
    fn take_for_shutdown(&mut self) -> Result<Vec<Arc<HostedAddon>>, String> {
        if self.loading || self.dispatches != 0 || self.stopping {
            return Err("Addon host is busy; retry after dispatch completes".into());
        }
        self.stopped = true;
        self.stopping = true;
        Ok(std::mem::take(&mut self.addons)
            .into_iter()
            .map(|entry| entry.addon)
            .collect())
    }
}
static HOST: OnceLock<Mutex<Host>> = OnceLock::new();
fn host() -> &'static Mutex<Host> {
    HOST.get_or_init(|| Mutex::new(Host::default()))
}
fn record(outcomes: Vec<(u32, mlua::Result<MultiValue>)>, actions: Vec<HostAction>) {
    let mut host = host().lock().unwrap_or_else(|error| error.into_inner());
    host.callbacks += outcomes.len() as u32;
    for (_, result) in outcomes {
        if let Err(error) = result {
            host.errors.push(error.to_string());
        }
    }
    host.queue.extend(actions);
}
fn record_foreign(result: Result<Response, String>) {
    let response = match result {
        Ok(response) => response,
        Err(message) => {
            error(message);
            return;
        }
    };
    let mut host = host().lock().unwrap_or_else(|error| error.into_inner());
    host.callbacks += response.callbacks.len() as u32;
    host.errors.extend(response.errors);
    host.errors.extend(
        response
            .callbacks
            .into_iter()
            .filter_map(|callback| callback.error),
    );
    host.queue
        .extend(response.actions.into_iter().map(|action| match action {
            vm::Action::GameInput {
                origin,
                mode,
                bytes,
            } => HostAction::GameInput {
                origin,
                mode,
                command: bytes,
            },
            vm::Action::WindowerCommand { bytes } => HostAction::Command(bytes),
            vm::Action::NativeCommand { bytes } => HostAction::NativeCommand(bytes),
            vm::Action::AshitaCommand { mode, bytes } => HostAction::AshitaCommand {
                mode,
                command: bytes,
            },
            vm::Action::Chat { mode, bytes } => HostAction::Chat {
                mode,
                message: bytes,
            },
        }));
}
fn error(message: String) {
    host()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .errors
        .push(message);
}
type CreatedAddon = (Arc<HostedAddon>, Option<Response>);
fn create_addon(
    root: &std::path::Path,
    runtime_path: &str,
    spec: &AddonSpec,
    provider: &mut Option<Arc<Provider>>,
) -> Result<CreatedAddon, String> {
    let name = spec.name.as_str();
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', ':', '\0']) {
        return Err("Invalid addon directory name".into());
    }
    let file = root
        .join("addons")
        .join(name)
        .join(format!("{name}.lua"))
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !file.starts_with(root) {
        return Err("Addon path escapes the selected runtime root".into());
    }
    if fs::metadata(&file)
        .map_err(|error| error.to_string())?
        .len()
        > 16 * 1024 * 1024
    {
        return Err("Addon source exceeds 16 MiB".into());
    }
    let source = fs::read_to_string(&file).map_err(|error| error.to_string())?;
    let directory = format!(
        "{}/",
        file.parent()
            .ok_or("Missing addon parent")?
            .to_string_lossy()
            .replace('\\', "/")
    );
    if spec.engine == LuaEngine::LuaJit {
        if provider.is_none() {
            let library = crate::native::options()
                .and_then(|options| options.luajit_provider.as_ref())
                .ok_or("Missing LuaJIT provider selection")?;
            library.verify()?;
            *provider = Some(Arc::new(unsafe { Provider::open(&library.path) }?));
        }
        let provider = provider.as_ref().ok_or("Missing LuaJIT provider")?.clone();
        let epoch = Instant::now();
        let response = provider.request(Request::new(Operation::Create {
            name: name.into(),
            runtime_path: runtime_path.into(),
            addon_path: directory,
            source,
            ecosystem: spec.ecosystem,
        }))?;
        if !response.ok {
            return Err(response.error.unwrap_or("Provider creation failed".into()));
        }
        let handle = response.handle.ok_or("Provider returned no addon handle")?;
        let addon = Arc::new(HostedAddon::Foreign {
            provider,
            handle,
            ecosystem: spec.ecosystem,
            epoch,
        });
        return Ok((addon, Some(response)));
    }
    Addon::from_source_with_ecosystem(name, runtime_path, &directory, &source, spec.ecosystem)
        .map(|addon| (Arc::new(HostedAddon::Local(addon)), None))
        .map_err(|error| error.to_string())
}
fn ensure_loaded() {
    {
        let mut host = host().lock().unwrap_or_else(|error| error.into_inner());
        if host.loaded || host.loading || host.stopped {
            return;
        }
        host.loading = true;
    }
    let result = (|| -> Result<(), String> {
        let settings = crate::native::settings().ok_or("Missing runtime settings")?;
        let mut names = settings
            .init_addons
            .split(';')
            .filter(|name| !name.is_empty())
            .map(|name| AddonSpec {
                name: name.into(),
                ecosystem: Ecosystem::Windower4,
                engine: LuaEngine::Lua51,
            })
            .collect::<Vec<_>>();
        if let Some(options) = crate::native::options() {
            for requested in &options.addons {
                if let Some(existing) = names
                    .iter_mut()
                    .find(|item| item.name.eq_ignore_ascii_case(&requested.name))
                {
                    *existing = requested.clone();
                } else {
                    names.push(requested.clone());
                }
            }
        }
        if names.is_empty() {
            return Ok(());
        }
        let root = PathBuf::from(&settings.windower_path)
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let mut provider: Option<Arc<Provider>> = None;
        for spec in names {
            let name = spec.name.as_str();
            let result = create_addon(&root, &settings.windower_path, &spec, &mut provider);
            match result {
                Ok((addon, initial)) => {
                    if let Some(response) = initial {
                        record_foreign(Ok(response));
                    }
                    addon.event("load");
                    host()
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .addons
                        .push(LoadedAddon {
                            spec: spec.clone(),
                            addon,
                        });
                }
                Err(message) => error(format!("{name}: {message}")),
            }
        }
        Ok(())
    })();
    if let Err(message) = result {
        error(message);
    }
    let mut host = host().lock().unwrap_or_else(|error| error.into_inner());
    host.loading = false;
    host.loaded = true;
}
struct Dispatch {
    addons: Vec<Arc<HostedAddon>>,
}
struct ReloadGuard;
impl Drop for ReloadGuard {
    fn drop(&mut self) {
        host().lock().unwrap_or_else(|e| e.into_inner()).loading = false;
    }
}
/// Stage the selected addon before replacing its VM. Source side effects such
/// as arbitrary file I/O cannot be rolled back, but failed staging retains the
/// original VM, handlers and schedules. Lua always executes outside host locks.
pub fn reload(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 256 || name.contains(['/', '\\', ':', '\0']) {
        return Err("Invalid addon name".into());
    }
    let mut plan = host()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .reserve_reload(name)?;
    let _guard = ReloadGuard;
    let settings = crate::native::settings().ok_or("Missing runtime settings")?;
    let root = PathBuf::from(&settings.windower_path)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let (addon, initial) = create_addon(
        &root,
        &settings.windower_path,
        &plan.spec,
        &mut plan.provider,
    )?;
    let previous = {
        let mut host = host().lock().unwrap_or_else(|e| e.into_inner());
        std::mem::replace(
            &mut host.addons[plan.index],
            LoadedAddon {
                spec: plan.spec,
                addon: addon.clone(),
            },
        )
    };
    previous.addon.event("unload");
    drop(previous);
    if let Some(response) = initial {
        record_foreign(Ok(response));
    }
    addon.event("load");
    let mut host = host().lock().unwrap_or_else(|e| e.into_inner());
    host.reloads = host.reloads.saturating_add(1);
    Ok(())
}
impl Dispatch {
    fn acquire() -> Option<Self> {
        let mut host = host().lock().unwrap_or_else(|error| error.into_inner());
        if host.stopped || host.loading {
            return None;
        }
        host.dispatches += 1;
        Some(Self {
            addons: host
                .addons
                .iter()
                .map(|entry| entry.addon.clone())
                .collect(),
        })
    }
}
impl Drop for Dispatch {
    fn drop(&mut self) {
        // Release VM references before announcing quiescence. Lua cleanup may
        // execute finalizers and must run outside the host bookkeeping lock.
        self.addons.clear();
        host()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .dispatches -= 1;
    }
}
pub fn shutdown() -> Result<(), String> {
    let addons = host()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take_for_shutdown()?;
    for addon in &addons {
        addon.event("unload");
    }
    drop(addons);
    host()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .stopping = false;
    Ok(())
}
pub fn on_frame() {
    ensure_loaded();
    let Some(dispatch) = Dispatch::acquire() else {
        return;
    };
    let index = {
        let mut host = host().lock().unwrap_or_else(|error| error.into_inner());
        host.frames += 1;
        host.frames
    };
    for addon in &dispatch.addons {
        addon.frame(index);
    }
}
fn input(name: &str, args: MultiValue, blocked_at: usize) -> bool {
    ensure_loaded();
    let Some(dispatch) = Dispatch::acquire() else {
        return false;
    };
    let mut blocked = false;
    for addon in &dispatch.addons {
        blocked = addon.input(name, args.clone(), blocked_at, blocked);
    }
    blocked
}
pub fn keyboard(dik: u32, pressed: bool, flags: u32) -> bool {
    input(
        "keyboard",
        MultiValue::from_vec(vec![
            Value::Integer(dik as i32),
            Value::Boolean(pressed),
            Value::Integer(flags as i32),
            Value::Boolean(false),
        ]),
        3,
    )
}
pub fn mouse(kind: u32, x: i32, y: i32, delta: i32) -> bool {
    input(
        "mouse",
        MultiValue::from_vec(vec![
            Value::Integer(kind as i32),
            Value::Integer(x),
            Value::Integer(y),
            Value::Integer(delta),
            Value::Boolean(false),
        ]),
        4,
    )
}
pub fn counter(which: u32) -> u32 {
    let host = host().lock().unwrap_or_else(|error| error.into_inner());
    match which {
        0 => host.frames,
        1 => host.addons.len() as u32,
        2 => host.callbacks,
        3 => host
            .queue
            .iter()
            .filter(|action| action.is_command())
            .count() as u32,
        4 => host
            .queue
            .iter()
            .filter(|action| matches!(action, HostAction::Chat { .. }))
            .count() as u32,
        5 => host.errors.len() as u32,
        6 => host
            .queue
            .iter()
            .filter(|action| matches!(action, HostAction::Command(_)))
            .count() as u32,
        7 => host
            .queue
            .iter()
            .filter(|action| matches!(action, HostAction::NativeCommand(_)))
            .count() as u32,
        8 => host
            .queue
            .iter()
            .filter(|action| matches!(action, HostAction::AshitaCommand { .. }))
            .count() as u32,
        9 => host
            .addons
            .iter()
            .filter(|entry| matches!(entry.addon.as_ref(), HostedAddon::Foreign { .. }))
            .count() as u32,
        10 => u32::from(host.stopped && !host.stopping),
        11 => host.reloads,
        _ => u32::MAX,
    }
}
pub fn command_index(bytes: &[u8]) -> u32 {
    host()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .queue
        .iter()
        .position(|action| match action {
            HostAction::Command(command)
            | HostAction::NativeCommand(command)
            | HostAction::GameInput { command, .. }
            | HostAction::AshitaCommand { command, .. } => command == bytes,
            _ => false,
        })
        .map(|index| index as u32)
        .unwrap_or(u32::MAX)
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    #[test]
    fn reload_reservation_rejects_dispatch_load_shutdown_and_unknown_addons() {
        let addon = Addon::from_source("A", "C:/fixture/", "C:/fixture/A/", "").unwrap();
        let mut host = Host {
            loaded: true,
            addons: vec![LoadedAddon {
                spec: AddonSpec {
                    name: "A".into(),
                    ecosystem: Ecosystem::Windower4,
                    engine: LuaEngine::Lua51,
                },
                addon: Arc::new(HostedAddon::Local(addon)),
            }],
            ..Default::default()
        };
        assert!(host.reserve_reload("missing").is_err());
        host.dispatches = 1;
        assert!(host.reserve_reload("A").is_err());
        host.dispatches = 0;
        host.loading = true;
        assert!(host.reserve_reload("A").is_err());
        host.loading = false;
        host.stopped = true;
        assert!(host.reserve_reload("A").is_err());
        host.stopped = false;
        let plan = host.reserve_reload("a").unwrap();
        assert_eq!(plan.index, 0);
        assert_eq!(plan.spec.name, "A");
        assert!(host.loading);
        assert_eq!(host.addons.len(), 1);
    }
    #[test]
    fn shutdown_requires_quiescence_and_permanently_stops_dispatch() {
        let mut host = Host {
            loading: true,
            ..Host::default()
        };
        assert!(host.take_for_shutdown().is_err());
        assert!(!host.stopped);
        host.loading = false;
        host.dispatches = 1;
        assert!(host.take_for_shutdown().is_err());
        host.dispatches = 0;
        assert!(host.take_for_shutdown().unwrap().is_empty());
        assert!(host.stopped && host.stopping);
        assert!(host.take_for_shutdown().is_err());
        host.stopping = false;
        assert!(host.take_for_shutdown().unwrap().is_empty());
    }
}
