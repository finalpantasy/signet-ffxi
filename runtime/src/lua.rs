//! Lua 5.1 addon hosting. Game-facing effects are queued for a future native
//! backend; successful queueing alone does not mean that an effect ran in FFXI.
use mlua::{Error, Function, Lua, MultiValue, Result, String as LuaString, Table, Value};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Instant,
};
use windower_rs::platform::Ecosystem;

#[derive(Clone, Debug, PartialEq)]
pub enum HostAction {
    Command(Vec<u8>),
    NativeCommand(Vec<u8>),
    GameInput {
        origin: Ecosystem,
        mode: i32,
        command: Vec<u8>,
    },
    AshitaCommand {
        mode: i32,
        command: Vec<u8>,
    },
    Chat {
        mode: i32,
        message: Vec<u8>,
    },
}
impl HostAction {
    pub fn is_command(&self) -> bool {
        matches!(
            self,
            Self::Command(_)
                | Self::NativeCommand(_)
                | Self::AshitaCommand { .. }
                | Self::GameInput { .. }
        )
    }
}
impl HostAction {
    pub fn into_protocol(self) -> windower_rs::vm_protocol::Action {
        use windower_rs::vm_protocol::Action;
        match self {
            Self::Command(bytes) => Action::WindowerCommand { bytes },
            Self::NativeCommand(bytes) => Action::NativeCommand { bytes },
            Self::GameInput {
                origin,
                mode,
                command,
            } => Action::GameInput {
                origin,
                mode,
                bytes: command,
            },
            Self::AshitaCommand { mode, command } => Action::AshitaCommand {
                mode,
                bytes: command,
            },
            Self::Chat { mode, message } => Action::Chat {
                mode,
                bytes: message,
            },
        }
    }
}

#[derive(Default)]
struct Events {
    next_id: u32,
    handlers: BTreeMap<u32, (String, Function)>,
    aliases: BTreeMap<(String, String), u32>,
}

pub struct Addon {
    lua: Lua,
    events: Arc<Mutex<Events>>,
    actions: Arc<Mutex<Vec<HostAction>>>,
    scheduler: Table,
    epoch: Instant,
    ecosystem: Ecosystem,
}

impl Addon {
    /// Execute a caller-selected addon with Lua's standard libraries available,
    /// including io/os/debug. Like stock Windower, this is not a script sandbox.
    /// The caller must authorize the selected source before invoking this API.
    pub fn from_source(
        name: &str,
        windower_path: &str,
        addon_path: &str,
        source: &str,
    ) -> Result<Self> {
        Self::from_source_with_ecosystem(
            name,
            windower_path,
            addon_path,
            source,
            Ecosystem::Windower4,
        )
    }

    pub fn from_source_with_ecosystem(
        name: &str,
        windower_path: &str,
        addon_path: &str,
        source: &str,
        ecosystem: Ecosystem,
    ) -> Result<Self> {
        if ecosystem == Ecosystem::Windower5 {
            return Err(Error::runtime(
                "Windower 5 package hosting is not implemented",
            ));
        }
        // Windower libraries use debug.getmetatable/setmetatable; ALL_SAFE omits
        // debug and C module loading and therefore breaks those libraries.
        let lua = unsafe { Lua::unsafe_new() };
        let scheduler = lua
            .load(include_str!("scheduler.lua"))
            .set_name("@windower/scheduler.lua")
            .eval()?;
        let events = Arc::new(Mutex::new(Events::default()));
        let actions = Arc::new(Mutex::new(Vec::new()));
        let windower = lua.create_table()?;
        windower.set("windower_path", windower_path)?;
        windower.set("addon_path", addon_path)?;
        let metadata = lua.create_table()?;
        metadata.set("name", name)?;
        lua.globals().set("_addon", metadata.clone())?;
        lua.globals().set("addon", metadata)?;

        let registered = Arc::clone(&events);
        windower.set(
            "register_event",
            lua.create_function(move |_, mut args: MultiValue| {
                let function = match args.pop_back() {
                    Some(Value::Function(function)) => function,
                    _ => {
                        return Err(Error::runtime(
                            "register_event requires event names followed by a function",
                        ));
                    }
                };
                if args.is_empty() {
                    return Err(Error::runtime(
                        "register_event requires at least one event name",
                    ));
                }
                let mut names = Vec::new();
                for value in args {
                    match value {
                        Value::String(name) => names.push(name.to_str()?.to_owned()),
                        _ => return Err(Error::runtime("Event names must be strings")),
                    }
                }
                let mut registry = registered
                    .lock()
                    .map_err(|_| Error::runtime("Event registry is poisoned"))?;
                let count = u32::try_from(names.len())
                    .map_err(|_| Error::runtime("Too many event names"))?;
                registry
                    .next_id
                    .checked_add(count)
                    .ok_or_else(|| Error::runtime("Event ID space exhausted"))?;
                let mut ids = Vec::new();
                for name in names {
                    registry.next_id += 1;
                    let id = registry.next_id;
                    registry.handlers.insert(id, (name, function.clone()));
                    // Lua 5.1 stores numbers as doubles; mlua's Integer is only
                    // 32 bits on i686, while every u32 remains exact in f64.
                    ids.push(Value::Number(f64::from(id)));
                }
                Ok(MultiValue::from_vec(ids))
            })?,
        )?;

        let registered = Arc::clone(&events);
        windower.set(
            "unregister_event",
            lua.create_function(move |_, ids: MultiValue| {
                let ids =
                    ids.into_iter()
                        .map(|value| match value {
                            Value::Integer(id) => u32::try_from(id)
                                .map_err(|_| Error::runtime("Invalid event handle")),
                            Value::Number(id)
                                if id.is_finite()
                                    && id.fract() == 0.0
                                    && (0.0..=u32::MAX as f64).contains(&id) =>
                            {
                                Ok(id as u32)
                            }
                            _ => Err(Error::runtime("Event handles must be integers")),
                        })
                        .collect::<Result<Vec<_>>>()?;
                let mut registry = registered
                    .lock()
                    .map_err(|_| Error::runtime("Event registry is poisoned"))?;
                for id in ids {
                    registry.handlers.remove(&id);
                    registry.aliases.retain(|_, value| *value != id);
                }
                Ok(())
            })?,
        )?;

        let queue = Arc::clone(&actions);
        windower.set(
            "send_command",
            lua.create_function(move |_, command: LuaString| {
                queue
                    .lock()
                    .map_err(|_| Error::runtime("Command queue is poisoned"))?
                    .push(HostAction::Command(command.as_bytes().to_vec()));
                Ok(())
            })?,
        )?;
        let queue = Arc::clone(&actions);
        windower.set(
            "add_to_chat",
            lua.create_function(move |_, (mode, message): (i32, LuaString)| {
                queue
                    .lock()
                    .map_err(|_| Error::runtime("Command queue is poisoned"))?
                    .push(HostAction::Chat {
                        mode,
                        message: message.as_bytes().to_vec(),
                    });
                Ok(())
            })?,
        )?;
        let chat = lua.create_table()?;
        chat.set(
            "input",
            input_function(&lua, &actions, Ecosystem::Windower4)?,
        )?;
        windower.set("chat", chat)?;
        install_native_api(&lua, &windower, &actions, ecosystem)?;
        if ecosystem == Ecosystem::Ashita4 {
            install_ashita_api(&lua, &events, &actions)?;
        }
        lua.globals().set("windower", windower)?;
        let package: Table = lua.globals().get("package")?;
        let existing: String = package.get("path")?;
        let root = windower_path
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_owned();
        let directory = addon_path
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_owned();
        package.set("path", format!("{directory}/?.lua;{directory}/?/init.lua;{root}/addons/libs/?.lua;{root}/addons/libs/?/init.lua;{existing}"))?;
        let addon = Self {
            lua,
            events,
            actions,
            scheduler,
            epoch: Instant::now(),
            ecosystem,
        };
        let normalized = windower_rs::lua_syntax::normalize(source.as_bytes());
        install_syntax_loader(&addon.lua)?;
        addon
            .lua
            .load(&normalized.bytes)
            .set_name(format!("@{addon_path}{name}.lua"))
            .exec()?;
        Ok(addon)
    }

    /// Dispatch a snapshot so handlers can register/unregister without keeping
    /// a mutable registry borrow across arbitrary Lua callbacks. Errors are
    /// isolated by handler; return all outcomes to the native host.
    pub fn emit(&self, name: &str, args: MultiValue) -> Vec<(u32, Result<MultiValue>)> {
        let snapshot = self
            .events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .handlers
            .iter()
            .filter(|(_, (event, _))| event == name)
            .map(|(id, (_, function))| (*id, function.clone()))
            .collect::<Vec<_>>();
        snapshot
            .into_iter()
            .map(|(id, function)| (id, self.dispatch(function, args.clone())))
            .collect()
    }

    fn dispatch(&self, function: Function, mut args: MultiValue) -> Result<MultiValue> {
        args.push_front(Value::Function(function));
        self.scheduler.get::<Function>("dispatch")?.call(args)
    }

    pub fn poll(&self) -> Result<Vec<String>> {
        self.poll_at(self.epoch.elapsed().as_secs_f64())
    }

    /// Advance an addon-local monotonic clock, also allowing deterministic
    /// scheduler replay without sleeping in tests. Never hold host locks here.
    pub fn poll_at(&self, seconds: f64) -> Result<Vec<String>> {
        let errors: Table = self.scheduler.get::<Function>("tick")?.call(seconds)?;
        errors.sequence_values::<String>().collect()
    }

    pub fn scheduled_count(&self) -> Result<usize> {
        self.scheduler.get::<Function>("pending")?.call(())
    }

    pub fn ecosystem(&self) -> Ecosystem {
        self.ecosystem
    }

    pub fn emit_frame(&self, index: u32) -> Vec<(u32, Result<MultiValue>)> {
        match self.ecosystem {
            Ecosystem::Windower4 => self.emit("prerender", MultiValue::new()),
            Ecosystem::Ashita4 => self.emit("d3d_present", MultiValue::new()),
            Ecosystem::Native => {
                let args = (|| -> Result<MultiValue> {
                    let event = self.lua.create_table()?;
                    event.set("index", index)?;
                    event.set("elapsed_seconds", self.epoch.elapsed().as_secs_f64())?;
                    Ok(MultiValue::from_vec(vec![Value::Table(event)]))
                })();
                match args {
                    Ok(args) => self.emit("frame", args),
                    Err(error) => vec![(0, Err(error))],
                }
            }
            Ecosystem::Windower5 => vec![],
        }
    }

    pub fn take_actions(&self) -> Vec<HostAction> {
        std::mem::take(
            &mut *self
                .actions
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        )
    }

    pub fn emit_blocking(
        &self,
        name: &str,
        mut args: MultiValue,
        blocked_at: usize,
        mut blocked: bool,
    ) -> (bool, Vec<(u32, Result<MultiValue>)>) {
        let snapshot = self
            .events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .handlers
            .iter()
            .filter(|(_, (event, _))| event == name)
            .map(|(id, (_, function))| (*id, function.clone()))
            .collect::<Vec<_>>();
        let mut outcomes = Vec::new();
        for (id, function) in snapshot {
            if let Some(value) = args.get_mut(blocked_at) {
                *value = Value::Boolean(blocked);
            }
            let result: Result<MultiValue> = self.dispatch(function, args.clone());
            if result
                .as_ref()
                .is_ok_and(|values| matches!(values.front(), Some(Value::Boolean(true))))
            {
                blocked = true;
            }
            outcomes.push((id, result));
        }
        (blocked, outcomes)
    }
    pub fn lua(&self) -> &Lua {
        &self.lua
    }
    pub fn handler_count(&self) -> usize {
        self.events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .handlers
            .len()
    }
}

fn install_native_api(
    lua: &Lua,
    legacy: &Table,
    actions: &Arc<Mutex<Vec<HostAction>>>,
    ecosystem: Ecosystem,
) -> Result<()> {
    let api = lua.create_table()?;
    api.set("api_version", 1)?;
    let runtime = lua.create_table()?;
    runtime.set("name", "windower-rs")?;
    runtime.set("ecosystem", ecosystem.name())?;
    runtime.set(
        "engine",
        if matches!(lua.globals().get::<Value>("jit")?, Value::Table(_)) {
            "luajit"
        } else {
            "lua51"
        },
    )?;
    runtime.set("game_integration_verified", false)?;
    api.set("runtime", runtime)?;
    let events = lua.create_table()?;
    events.set("on", legacy.get::<Function>("register_event")?)?;
    events.set("off", legacy.get::<Function>("unregister_event")?)?;
    api.set("events", events)?;
    let commands = lua.create_table()?;
    commands.set("input", input_function(lua, actions, Ecosystem::Native)?)?;
    let queue = Arc::clone(actions);
    commands.set(
        "queue",
        lua.create_function(move |_, command: LuaString| {
            queue
                .lock()
                .map_err(|_| Error::runtime("Command queue is poisoned"))?
                .push(HostAction::NativeCommand(command.as_bytes().to_vec()));
            Ok(())
        })?,
    )?;
    api.set("commands", commands)?;
    let chat = lua.create_table()?;
    chat.set("queue", legacy.get::<Function>("add_to_chat")?)?;
    api.set("chat", chat)?;
    let capabilities = lua.create_table()?;
    capabilities.set(
        "supports",
        lua.create_function(|_, id: String| Ok(windower_rs::platform::supports_api(&id)))?,
    )?;
    capabilities.set(
        "list",
        lua.create_function(|lua, ()| {
            let list = lua.create_table()?;
            for (index, item) in windower_rs::platform::CAPABILITIES.iter().enumerate() {
                let capability = lua.create_table()?;
                capability.set("id", item.id)?;
                capability.set("callable", item.api_callable)?;
                capability.set("in_game_verified", item.in_game_verified)?;
                capability.set("detail", item.detail)?;
                list.set(index + 1, capability)?;
            }
            Ok(list)
        })?,
    )?;
    api.set("capabilities", capabilities)?;
    lua.globals().set("ffxi", api)
}

/// Stock Lua's filesystem searcher cannot parse Windower literal-method syntax.
/// Replace only that searcher; native/trusted-addon policy is otherwise unchanged.
fn install_syntax_loader(lua: &Lua) -> Result<()> {
    let package: Table = lua.globals().get("package")?;
    let loaders: Table = package.get("loaders")?;
    loaders.set(
        2,
        lua.create_function(|lua, name: String| {
            use std::io::Read;
            let package: Table = lua.globals().get("package")?;
            let paths: String = package.get("path")?;
            let mut errors = String::new();
            for template in paths.split(';').filter(|p| !p.is_empty()) {
                let path = template.replace('?', &name.replace('.', "/"));
                match std::fs::File::open(&path) {
                    Ok(file) => {
                        let mut bytes = Vec::new();
                        file.take(windower_rs::addon_profile::SOURCE_LIMIT + 1)
                            .read_to_end(&mut bytes)
                            .map_err(Error::external)?;
                        if bytes.len() as u64 > windower_rs::addon_profile::SOURCE_LIMIT {
                            return Err(Error::runtime("Lua module source limit exceeded"));
                        }
                        let normalized = windower_rs::lua_syntax::normalize(&bytes);
                        return lua
                            .load(&normalized.bytes)
                            .set_name(format!("@{path}"))
                            .into_function()
                            .map(Value::Function);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        errors.push_str(&format!("\n\tno file '{path}'"));
                    }
                    Err(error) => return Err(Error::external(error)),
                }
            }
            Ok(Value::String(lua.create_string(errors)?))
        })?,
    )
}

fn input_function(
    lua: &Lua,
    actions: &Arc<Mutex<Vec<HostAction>>>,
    origin: Ecosystem,
) -> Result<Function> {
    let queue = actions.clone();
    lua.create_function(move |_, command: LuaString| {
        let command = command.as_bytes().to_vec();
        windower_rs::commands::Input {
            origin,
            mode: 1,
            bytes: command.clone(),
        }
        .validate()
        .map_err(Error::runtime)?;
        queue
            .lock()
            .map_err(|_| Error::runtime("Command queue is poisoned"))?
            .push(HostAction::GameInput {
                origin,
                mode: 1,
                command,
            });
        Ok(())
    })
}

fn install_ashita_api(
    lua: &Lua,
    registry: &Arc<Mutex<Events>>,
    actions: &Arc<Mutex<Vec<HostAction>>>,
) -> Result<()> {
    let ashita = lua.create_table()?;
    let events = lua.create_table()?;
    let registered = Arc::clone(registry);
    events.set(
        "register",
        lua.create_function(
            move |_, (event, alias, callback): (String, String, Function)| {
                let mut registry = registered
                    .lock()
                    .map_err(|_| Error::runtime("Event registry is poisoned"))?;
                let key = (event.clone(), alias);
                if registry.aliases.contains_key(&key) {
                    return Ok(false);
                }
                let id = registry
                    .next_id
                    .checked_add(1)
                    .ok_or_else(|| Error::runtime("Event ID space exhausted"))?;
                registry.next_id = id;
                registry.handlers.insert(id, (event, callback));
                registry.aliases.insert(key, id);
                Ok(true)
            },
        )?,
    )?;
    let registered = Arc::clone(registry);
    events.set(
        "unregister",
        lua.create_function(move |_, (event, alias): (String, String)| {
            let mut registry = registered
                .lock()
                .map_err(|_| Error::runtime("Event registry is poisoned"))?;
            match registry.aliases.remove(&(event, alias)) {
                Some(id) => {
                    registry.handlers.remove(&id);
                    Ok(true)
                }
                None => Ok(false),
            }
        })?,
    )?;
    ashita.set("events", events)?;
    lua.globals().set("ashita", ashita)?;
    let chat = lua.create_table()?;
    let queue = Arc::clone(actions);
    chat.set(
        "QueueCommand",
        lua.create_function(move |_, (_self, mode, command): (Value, i32, LuaString)| {
            queue
                .lock()
                .map_err(|_| Error::runtime("Command queue is poisoned"))?
                .push(HostAction::AshitaCommand {
                    mode,
                    command: command.as_bytes().to_vec(),
                });
            Ok(())
        })?,
    )?;
    let core = lua.create_table()?;
    let chat_key = lua.create_registry_value(chat)?;
    core.set(
        "GetChatManager",
        lua.create_function(move |lua, _self: Value| lua.registry_value::<Table>(&chat_key))?,
    )?;
    lua.globals().set("AshitaCore", core)
}

impl Drop for Addon {
    fn drop(&mut self) {
        if let Ok(clear) = self.scheduler.get::<Function>("clear") {
            let _ = clear.call::<()>(());
        }
        // Registered Function handles retain their Lua VM. The VM's register
        // closure retains this registry; break that cycle when unloading, also
        // when source execution fails after registering a handler.
        self.events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .handlers
            .clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_literal_method_syntax_works_in_entries_and_required_files() {
        let directory = std::env::temp_dir().join(format!(
            "lua-dialect-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(
            directory.join("literal.lua"),
            b"-- copyright \xa9\nreturn 'module %s':format('ok')",
        )
        .unwrap();
        let path = format!("{}/", directory.display());
        Addon::from_source(
            "dialect",
            &path,
            &path,
            "assert(require('literal')=='module ok'); assert('entry %s':format('ok')=='entry ok'); local v=function() if true then return function()return 'ok'end() end end(); assert(v=='ok')",
        )
        .unwrap();
    }
    #[test]
    fn direct_game_input_is_distinct_from_console_commands_and_validates_before_queueing() {
        let addon = Addon::from_source(
            "input",
            "C:/Test/",
            "C:/Test/input/",
            r#"
            windower.chat.input('/echo literal;semicolon')
            ffxi.commands.input('/echo native')
            assert(not pcall(windower.chat.input, 'bad' .. string.char(0)))
            assert(not pcall(ffxi.commands.input, string.rep('a', 1024)))
        "#,
        )
        .unwrap();
        assert_eq!(
            addon
                .take_actions()
                .into_iter()
                .map(HostAction::into_protocol)
                .collect::<Vec<_>>(),
            vec![
                windower_rs::vm_protocol::Action::GameInput {
                    origin: Ecosystem::Windower4,
                    mode: 1,
                    bytes: b"/echo literal;semicolon".to_vec()
                },
                windower_rs::vm_protocol::Action::GameInput {
                    origin: Ecosystem::Native,
                    mode: 1,
                    bytes: b"/echo native".to_vec()
                },
            ]
        );
    }

    #[test]
    fn native_api_exposes_capabilities_and_keeps_command_origin() {
        let addon = Addon::from_source_with_ecosystem("native", "C:/Test/", "C:/Test/native/", r#"
            assert(ffxi.api_version == 1)
            assert(ffxi.runtime.ecosystem == 'native' and not ffxi.runtime.game_integration_verified)
            assert(ffxi.capabilities.supports('api.events'))
            assert(not ffxi.capabilities.supports('game.state'))
            assert(#ffxi.capabilities.list() > 5)
            ffxi.events.on('frame', function(e)
                assert(e.index == 9 and type(e.elapsed_seconds) == 'number')
                ffxi.commands.queue('native\0command')
                ffxi.chat.queue(207, 'queued')
            end)
        "#, Ecosystem::Native).unwrap();
        assert!(addon.emit_frame(9).iter().all(|(_, result)| result.is_ok()));
        assert_eq!(
            addon.take_actions(),
            vec![
                HostAction::NativeCommand(b"native\0command".to_vec()),
                HostAction::Chat {
                    mode: 207,
                    message: b"queued".to_vec()
                }
            ]
        );
    }

    #[test]
    fn ashita_event_aliases_are_scoped_and_commands_preserve_mode() {
        let addon = Addon::from_source_with_ecosystem("ashita", "C:/Test/", "C:/Test/ashita/", r#"
            assert(addon.name == 'ashita' and type(ashita.events.register) == 'function')
            assert(ashita.events.register('d3d_present', 'callback', function()
                AshitaCore:GetChatManager():QueueCommand(1, '/echo first\0binary')
                assert(ashita.events.unregister('d3d_present', 'callback'))
            end))
            assert(not ashita.events.register('d3d_present', 'callback', function() error('duplicate') end))
            assert(ashita.events.register('load', 'callback', function() end))
            assert(not ashita.events.unregister('missing', 'callback'))
        "#, Ecosystem::Ashita4).unwrap();
        assert!(addon.emit_frame(1).iter().all(|(_, result)| result.is_ok()));
        assert_eq!(
            addon.take_actions(),
            vec![HostAction::AshitaCommand {
                mode: 1,
                command: b"/echo first\0binary".to_vec()
            }]
        );
        assert!(addon.emit_frame(2).is_empty());
        let registry = Arc::downgrade(&addon.events);
        drop(addon);
        assert!(registry.upgrade().is_none());
    }

    #[test]
    fn unimplemented_windower5_engine_is_rejected_before_source_runs() {
        assert!(
            Addon::from_source_with_ecosystem(
                "v5",
                "C:/Test/",
                "C:/Test/v5/",
                "error('source must not run')",
                Ecosystem::Windower5
            )
            .err()
            .unwrap()
            .to_string()
            .contains("package hosting is not implemented")
        );
    }

    #[test]
    fn scheduled_work_sleeps_cancels_and_preserves_equal_time_order() {
        let addon = Addon::from_source(
            "clock",
            "C:/Test/",
            "C:/Test/addons/clock/",
            r#"
            local canceled = coroutine.schedule(function() error('must not run') end, 1)
            coroutine.close(canceled)
            assert(not coroutine.resume(canceled))
            coroutine.schedule(function()
                windower.send_command('first')
                coroutine.sleep(2)
                windower.send_command('awake\0binary')
            end, 1)
            coroutine.schedule(function() windower.send_command('second') end, 1)
            coroutine.schedule(function() error('scheduled error') end, 1)
            coroutine.schedule(function() windower.send_command('after error') end, 1)
        "#,
        )
        .unwrap();
        assert_eq!(addon.scheduled_count().unwrap(), 4);
        assert!(addon.poll_at(0.99).unwrap().is_empty());
        assert!(addon.take_actions().is_empty());
        let errors = addon.poll_at(1.0).unwrap();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("scheduled error"));
        assert_eq!(
            addon.take_actions(),
            vec![
                HostAction::Command(b"first".to_vec()),
                HostAction::Command(b"second".to_vec()),
                HostAction::Command(b"after error".to_vec())
            ]
        );
        assert_eq!(addon.scheduled_count().unwrap(), 1);
        addon.poll_at(3.0).unwrap();
        assert_eq!(
            addon.take_actions(),
            vec![HostAction::Command(b"awake\0binary".to_vec())]
        );
        assert_eq!(addon.scheduled_count().unwrap(), 0);
        assert!(addon.poll_at(2.0).is_err());
    }

    #[test]
    fn event_sleep_and_zero_delay_wait_for_the_next_tick() {
        let addon = Addon::from_source(
            "clock",
            "C:/Test/",
            "C:/Test/addons/clock/",
            r#"
            windower.register_event('load', function()
                windower.send_command('load')
                coroutine.sleep(0)
                windower.send_command('resume')
                coroutine.schedule(function() windower.send_command('next tick') end, 0)
            end)
        "#,
        )
        .unwrap();
        assert!(addon.emit("load", MultiValue::new())[0].1.is_ok());
        assert_eq!(
            addon.take_actions(),
            vec![HostAction::Command(b"load".to_vec())]
        );
        addon.poll_at(0.0).unwrap();
        assert_eq!(
            addon.take_actions(),
            vec![HostAction::Command(b"resume".to_vec())]
        );
        addon.poll_at(0.0).unwrap();
        assert_eq!(
            addon.take_actions(),
            vec![HostAction::Command(b"next tick".to_vec())]
        );
    }

    #[test]
    fn due_jobs_can_cancel_each_other_and_manual_resume_does_not_duplicate_work() {
        let addon = Addon::from_source(
            "clock",
            "C:/Test/",
            "C:/Test/addons/clock/",
            r#"
            local later
            coroutine.schedule(function() coroutine.close(later) end, 0)
            later = coroutine.schedule(function() error('canceled due job') end, 0)
            local manual = coroutine.schedule(function()
                windower.send_command('manual')
                coroutine.sleep(2)
                windower.send_command('resumed once')
            end, 0)
            assert(coroutine.resume(manual))
            local stop = coroutine.schedule(function()
                coroutine.close(coroutine.running())
                coroutine.sleep(0)
                error('closed self ran again')
            end, 0)
        "#,
        )
        .unwrap();
        assert_eq!(
            addon.take_actions(),
            vec![HostAction::Command(b"manual".to_vec())]
        );
        assert!(addon.poll_at(0.0).unwrap().is_empty());
        assert_eq!(addon.scheduled_count().unwrap(), 1);
        assert!(addon.poll_at(2.0).unwrap().is_empty());
        assert_eq!(
            addon.take_actions(),
            vec![HostAction::Command(b"resumed once".to_vec())]
        );
        assert_eq!(addon.scheduled_count().unwrap(), 0);
    }

    #[test]
    fn standard_coroutine_results_and_delay_validation_are_preserved() {
        Addon::from_source("clock", "C:/Test/", "C:/Test/addons/clock/", r#"
            local co = coroutine.create(function() coroutine.yield(1, nil, 3); return true, nil, 5 end)
            local ok, a, b, c = coroutine.resume(co)
            assert(ok and a == 1 and b == nil and c == 3)
            ok, a, b, c = coroutine.resume(co)
            assert(ok and a == true and b == nil and c == 5)
            local wrap = coroutine.wrap(function() return 7, nil, 9 end)
            a, b, c = wrap()
            assert(a == 7 and b == nil and c == 9)
            for _, bad in ipairs({-1, math.huge, 0/0, '1', false}) do
                assert(not pcall(coroutine.schedule, function() end, bad))
            end
            assert(not pcall(coroutine.sleep, 0))
            assert(not pcall(coroutine.close, {}))
        "#).unwrap();
    }

    #[test]
    fn require_finds_the_selected_runtime_libraries() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .find(|path| path.join("tests/fixtures/runtime-root").is_dir())
            .expect("Missing project fixture root")
            .join("tests/fixtures/runtime-root");
        let addon_path = root.join("addons/RustFixture");
        let addon = Addon::from_source(
            "library",
            &root.to_string_lossy(),
            &addon_path.to_string_lossy(),
            "windower.send_command(require('native_fixture').message)",
        )
        .unwrap();
        assert_eq!(
            addon.take_actions(),
            vec![HostAction::Command(b"fixture library".to_vec())]
        );
    }

    #[test]
    fn lua51_callbacks_and_binary_messages_work() {
        let addon = Addon::from_source(
            "fixture",
            "C:/Test/",
            "C:/Test/addons/fixture/",
            r#"
            assert(_VERSION == 'Lua 5.1')
            assert(type(debug.getmetatable) == 'function')
            first, second = windower.register_event('load', 'unload', function()
                windower.send_command('echo hello')
                windower.add_to_chat(207, string.char(0x81, 0x40))
            end)
        "#,
        )
        .unwrap();
        assert_eq!(addon.handler_count(), 2);
        let outcomes = addon.emit("load", MultiValue::new());
        assert!(outcomes.iter().all(|(_, result)| result.is_ok()));
        assert_eq!(
            addon.take_actions(),
            vec![
                HostAction::Command(b"echo hello".to_vec()),
                HostAction::Chat {
                    mode: 207,
                    message: vec![0x81, 0x40]
                }
            ]
        );
        addon
            .lua()
            .load("windower.unregister_event(first, second)")
            .exec()
            .unwrap();
        assert!(addon.emit("load", MultiValue::new()).is_empty());
    }

    #[test]
    fn handlers_can_unregister_themselves_and_errors_are_isolated() {
        let addon = Addon::from_source("fixture", "C:/Test/", "C:/Test/fixture/", r#"
            self_id = windower.register_event('frame', function() windower.unregister_event(self_id); error('fixture error') end)
            windower.register_event('frame', function() windower.send_command('echo still ran') end)
        "#).unwrap();
        let outcomes = addon.emit("frame", MultiValue::new());
        assert_eq!(outcomes.len(), 2);
        assert!(outcomes[0].1.is_err());
        assert!(outcomes[1].1.is_ok());
        assert_eq!(addon.handler_count(), 1);
        assert_eq!(
            addon.take_actions(),
            vec![HostAction::Command(b"echo still ran".to_vec())]
        );
    }

    #[test]
    fn separate_addons_do_not_share_lua_globals_or_handlers() {
        let first = Addon::from_source(
            "a",
            "C:/Test/",
            "C:/Test/a/",
            "private_value = 42; windower.register_event('load', function() end)",
        )
        .unwrap();
        let second = Addon::from_source(
            "b",
            "C:/Test/",
            "C:/Test/b/",
            "assert(private_value == nil)",
        )
        .unwrap();
        assert_eq!(first.handler_count(), 1);
        assert_eq!(second.handler_count(), 0);
    }

    #[test]
    fn invalid_registration_does_not_partially_register_events() {
        let addon = Addon::from_source(
            "fixture",
            "C:/Test/",
            "C:/Test/fixture/",
            "assert(not pcall(windower.register_event, 'load', 23, function() end))",
        )
        .unwrap();
        assert_eq!(addon.handler_count(), 0);
    }

    #[test]
    fn unloading_releases_vm_and_callback_registry() {
        let addon = Addon::from_source(
            "fixture",
            "C:/Test/",
            "C:/Test/fixture/",
            "windower.register_event('load', function() end)",
        )
        .unwrap();
        let registry = Arc::downgrade(&addon.events);
        drop(addon);
        assert!(registry.upgrade().is_none());
    }

    #[test]
    fn addon_can_move_to_another_host_thread() {
        let addon = Addon::from_source(
            "fixture",
            "C:/Test/",
            "C:/Test/fixture/",
            "windower.register_event('frame',function() windower.send_command('echo thread') end)",
        )
        .unwrap();
        let actions = std::thread::spawn(move || {
            assert!(
                addon
                    .emit("frame", MultiValue::new())
                    .iter()
                    .all(|(_, result)| result.is_ok())
            );
            addon.take_actions()
        })
        .join()
        .unwrap();
        assert_eq!(actions, vec![HostAction::Command(b"echo thread".to_vec())]);
    }

    #[test]
    fn later_input_handlers_receive_the_blocked_state() {
        let addon=Addon::from_source("fixture","C:/Test/","C:/Test/fixture/",r#"
            windower.register_event('keyboard',function(dik,pressed,flags,blocked) assert(not blocked);return true end)
            windower.register_event('keyboard',function(dik,pressed,flags,blocked) assert(blocked);return false end)
        "#).unwrap();
        let (blocked, outcomes) = addon.emit_blocking(
            "keyboard",
            MultiValue::from_vec(vec![
                Value::Integer(30),
                Value::Boolean(true),
                Value::Integer(0),
                Value::Boolean(false),
            ]),
            3,
            false,
        );
        assert!(blocked);
        assert_eq!(outcomes.len(), 2);
        assert!(outcomes.iter().all(|(_, result)| result.is_ok()));
    }
}
