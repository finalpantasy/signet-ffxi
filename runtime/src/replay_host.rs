//! Unmodified source replay on the actual Lua adapter, with explicit simulated services.
use crate::lua::Addon;
use mlua::{Error, Lua, MultiValue, Table, Value, VmState};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, AtomicU64, Ordering},
    },
};
use windower_rs::{addon_profile::SOURCE_LIMIT, platform::Ecosystem};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    pub schema_version: u32,
    pub name: String,
    pub ecosystem: Ecosystem,
    pub source: PathBuf,
    pub sha256: String,
    #[serde(default)]
    pub world: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub libraries: BTreeMap<String, PinnedLibrary>,
    pub steps: Vec<Step>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedLibrary {
    pub source: PathBuf,
    pub sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub at_ms: u64,
    pub event: String,
    #[serde(default)]
    pub arguments: Vec<serde_json::Value>,
}
#[derive(Default, Serialize)]
struct Trace {
    calls: Vec<(String, String)>,
    unsupported: Vec<String>,
    printed: Vec<String>,
    packet_intents: Vec<serde_json::Value>,
}
fn record(trace: &Arc<Mutex<Trace>>, name: &str, value: &str) -> mlua::Result<()> {
    let mut log = trace
        .lock()
        .map_err(|_| Error::runtime("Replay trace poisoned"))?;
    if log.calls.len() >= 512 || value.len() > 4096 {
        return Err(Error::runtime("Replay trace limit"));
    }
    log.calls.push((name.into(), value.into()));
    Ok(())
}
fn missing(trace: &Arc<Mutex<Trace>>, api: &str) -> mlua::Result<Value> {
    let mut log = trace
        .lock()
        .map_err(|_| Error::runtime("Replay trace poisoned"))?;
    if log.unsupported.len() >= 512 || api.len() > 4096 {
        return Err(Error::runtime("Replay missing-API limit"));
    }
    log.unsupported.push(api.into());
    Err(Error::runtime(format!("unsupported_api:{api}")))
}
fn value(lua: &Lua, json: &serde_json::Value, depth: usize) -> mlua::Result<Value> {
    if depth > 16 {
        return Err(Error::runtime("Replay value nesting limit"));
    }
    Ok(match json {
        serde_json::Value::Null => Value::Nil,
        serde_json::Value::Bool(b) => Value::Boolean(*b),
        serde_json::Value::Number(n) => Value::Number(
            n.as_f64()
                .ok_or_else(|| Error::runtime("Invalid replay number"))?,
        ),
        serde_json::Value::String(s) => Value::String(lua.create_string(s)?),
        serde_json::Value::Array(values) => {
            if values.len() > 512 {
                return Err(Error::runtime("Replay array limit"));
            }
            let t = lua.create_table()?;
            for (i, v) in values.iter().enumerate() {
                t.set(i + 1, value(lua, v, depth + 1)?)?;
            }
            Value::Table(t)
        }
        serde_json::Value::Object(values) => {
            if values.len() > 512 {
                return Err(Error::runtime("Replay map limit"));
            }
            let t = lua.create_table()?;
            for (k, v) in values {
                t.set(k.as_str(), value(lua, v, depth + 1)?)?;
            }
            Value::Table(t)
        }
    })
}
fn snapshot(v: Value, depth: usize) -> serde_json::Value {
    if depth > 12 {
        return serde_json::Value::String("<depth-limit>".into());
    }
    match v {
        Value::Nil => serde_json::Value::Null,
        Value::Boolean(b) => b.into(),
        Value::Integer(i) => i.into(),
        Value::Number(n) => serde_json::json!(n),
        Value::String(s) => String::from_utf8_lossy(&s.as_bytes()).into_owned().into(),
        Value::Table(t) => {
            let mut map = serde_json::Map::new();
            for (k, v) in t.pairs::<Value, Value>().filter_map(Result::ok).take(512) {
                let key = match k {
                    Value::String(s) => s.to_string_lossy(),
                    Value::Integer(i) => i.to_string(),
                    Value::Number(n) => n.to_string(),
                    _ => continue,
                };
                if !matches!(v, Value::Function(_)) {
                    map.insert(key, snapshot(v, depth + 1));
                }
            }
            serde_json::Value::Object(map)
        }
        _ => "<unsupported-value>".into(),
    }
}
fn guard(lua: &Lua, table: &Table, prefix: &str, trace: &Arc<Mutex<Trace>>) -> mlua::Result<()> {
    let meta = lua.create_table()?;
    let prefix = prefix.to_owned();
    let trace = Arc::clone(trace);
    meta.set(
        "__index",
        lua.create_function(move |_, (_, key): (Table, String)| {
            let api = format!("{prefix}.{key}");
            missing(&trace, &api)
        })?,
    )?;
    table.set_metatable(Some(meta))?;
    Ok(())
}
pub fn run(fixture: Fixture) -> Result<serde_json::Value, String> {
    use std::io::Read;
    if fixture.schema_version != 1 || fixture.steps.len() > 256 || fixture.name.len() > 256 {
        return Err("Invalid replay fixture bounds".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&fixture.source)
        .map_err(|e| e.to_string())?
        .take(SOURCE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > SOURCE_LIMIT || windower_rs::packages::digest(&bytes) != fixture.sha256
    {
        return Err("Replay source size/digest mismatch".into());
    }
    if fixture.libraries.len() > 32 {
        return Err("Replay library count limit".into());
    }
    let mut libraries = BTreeMap::new();
    let mut library_bytes = 0usize;
    for (name, pin) in &fixture.libraries {
        if name.is_empty()
            || name.len() > 128
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-/".contains(&b))
        {
            return Err("Invalid replay library name".into());
        }
        let mut data = Vec::new();
        std::fs::File::open(&pin.source)
            .map_err(|e| e.to_string())?
            .take(SOURCE_LIMIT + 1)
            .read_to_end(&mut data)
            .map_err(|e| e.to_string())?;
        library_bytes += data.len();
        if data.len() as u64 > SOURCE_LIMIT
            || library_bytes > 4 * 1024 * 1024
            || windower_rs::packages::digest(&data) != pin.sha256
        {
            return Err(format!("Replay library size/digest mismatch: {name}"));
        }
        libraries.insert(name.clone(), data);
    }
    let source = String::from_utf8(bytes).map_err(|_| "Replay source is not UTF-8")?;
    let addon = Addon::from_source_with_ecosystem(&fixture.name, "", "", "", fixture.ecosystem)
        .map_err(|e| e.to_string())?;
    let lua = addon.lua();
    lua.set_memory_limit(32 * 1024 * 1024)
        .map_err(|e| e.to_string())?;
    let fuel = Arc::new(AtomicU32::new(200));
    let hook = Arc::clone(&fuel);
    lua.set_hook(
        mlua::HookTriggers::new().every_nth_instruction(10000),
        move |_, _| {
            if hook
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
                .is_err()
            {
                Err(Error::runtime("Replay instruction budget exceeded"))
            } else {
                Ok(VmState::Continue)
            }
        },
    )
    .map_err(|e| e.to_string())?;
    let trace = Arc::new(Mutex::new(Trace::default()));
    let fonts = Arc::new(Mutex::new(Vec::<Table>::new()));
    let clock = Arc::new(AtomicU64::new(0));
    let install = (|| -> mlua::Result<()> {
        let record_trace = Arc::clone(&trace);
        lua.globals().set(
            "__replay_record",
            lua.create_function(move |_, (name, text): (String, String)| {
                record(&record_trace, &name, &text)
            })?,
        )?;
        let missing_trace = Arc::clone(&trace);
        lua.globals().set(
            "__replay_unsupported",
            lua.create_function(move |_, api: String| missing(&missing_trace, &api))?,
        )?;
        let font_trace = Arc::clone(&trace);
        let font_list = Arc::clone(&fonts);
        lua.globals().set(
            "__replay_font",
            lua.create_function(move |lua, settings: Table| {
                record(&font_trace, "fonts.new", "")?;
                let font = lua.create_table()?;
                for pair in settings.pairs::<Value, Value>() {
                    let (k, v) = pair?;
                    font.set(k, v)?;
                }
                font.set("destroyed", false)?;
                let apply_trace = Arc::clone(&font_trace);
                font.set(
                    "apply",
                    lua.create_function(move |_, (font, settings): (Table, Table)| {
                        record(&apply_trace, "fonts.apply", "")?;
                        for pair in settings.pairs::<Value, Value>() {
                            let (k, v) = pair?;
                            font.set(k, v)?;
                        }
                        Ok(())
                    })?,
                )?;
                let destroy_trace = Arc::clone(&font_trace);
                font.set(
                    "destroy",
                    lua.create_function(move |_, font: Table| {
                        record(&destroy_trace, "fonts.destroy", "")?;
                        font.set("destroyed", true)
                    })?,
                )?;
                font_list
                    .lock()
                    .map_err(|_| Error::runtime("Replay fonts poisoned"))?
                    .push(font.clone());
                Ok(font)
            })?,
        )?;
        let print_trace = Arc::clone(&trace);
        lua.globals().set(
            "print",
            lua.create_function(move |lua, args: MultiValue| {
                let stringify: mlua::Function = lua.globals().get("tostring")?;
                let mut words = Vec::new();
                for arg in args {
                    words.push(stringify.call::<String>(arg)?);
                }
                let text = words.join("\t");
                record(&print_trace, "print", &text)?;
                print_trace
                    .lock()
                    .map_err(|_| Error::runtime("Replay trace poisoned"))?
                    .printed
                    .push(text);
                Ok(())
            })?,
        )?;
        let legacy: Table = lua.globals().get("windower")?;
        lua.globals().set(
            "__replay_resources",
            value(
                lua,
                fixture
                    .world
                    .get("resources")
                    .unwrap_or(&serde_json::Value::Null),
                0,
            )?,
        )?;
        lua.globals().set(
            "__replay_capture_cancel",
            fixture
                .world
                .get("capture_cancel_packets")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        )?;
        let packet_trace = Arc::clone(&trace);
        lua.globals().set("__replay_packet_capture",lua.create_function(move |_,(direction,id,buff):(String,u32,u32)|{
            if direction!="outgoing" || id!=0xf1 || buff>u16::MAX as u32 {return Err(Error::runtime("Unsupported semantic packet fixture"));}
            record(&packet_trace,"packets.capture_semantic_only",&buff.to_string())?;
            packet_trace.lock().map_err(|_|Error::runtime("Replay trace poisoned"))?.packet_intents.push(
                serde_json::json!({"direction":direction,"id":id,"fields":{"Buff":buff},"encoding":"NOT_ENCODED"}));
            Ok(())
        })?)?;
        let matching_trace = Arc::clone(&trace);
        legacy.set(
            "wc_match",
            lua.create_function(move |_, (text, pattern): (String, String)| {
                if !text.is_ascii()
                    || !pattern.is_ascii()
                    || text.len() > 1024
                    || pattern.len() > 256
                    || pattern.contains(['*', '?'])
                {
                    missing(&matching_trace, "windower.wc_match:ASCII_literals_only")?;
                }
                Ok(text.eq_ignore_ascii_case(&pattern))
            })?,
        )?;
        let game = lua.create_table()?;
        for (api, key) in [
            ("get_player", "player"),
            ("get_info", "info"),
            ("get_party", "party"),
            ("get_items", "items"),
        ] {
            let data = fixture
                .world
                .get(key)
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let call_trace = Arc::clone(&trace);
            let name = format!("windower.ffxi.{api}");
            game.set(
                api,
                lua.create_function(move |lua, _args: MultiValue| {
                    record(&call_trace, &name, "")?;
                    value(lua, &data, 0)
                })?,
            )?;
        }
        let targets = fixture
            .world
            .get("targets")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let target_trace = Arc::clone(&trace);
        game.set(
            "get_mob_by_target",
            lua.create_function(move |lua, target: String| {
                if target.len() > 32 {
                    return Err(Error::runtime("Replay target name limit"));
                }
                record(&target_trace, "windower.ffxi.get_mob_by_target", &target)?;
                value(
                    lua,
                    targets.get(&target).unwrap_or(&serde_json::Value::Null),
                    0,
                )
            })?,
        )?;
        guard(lua, &game, "windower.ffxi", &trace)?;
        legacy.set("ffxi", game)?;
        let ipc = Arc::clone(&trace);
        legacy.set(
            "send_ipc_message",
            lua.create_function(move |_, text: String| record(&ipc, "ipc.capture", &text))?,
        )?;
        legacy.set(
            "convert_auto_trans",
            lua.create_function(|_, s: String| Ok(s))?,
        )?;
        guard(lua, &legacy, "windower", &trace)?;
        if fixture.ecosystem == Ecosystem::Ashita4 {
            let ashita: Table = lua.globals().get("ashita")?;
            let memory = lua.create_table()?;
            guard(lua, &memory, "ashita.memory", &trace)?;
            ashita.set("memory", memory)?;
            guard(lua, &ashita, "ashita", &trace)?;
        }
        let os = lua.create_table()?;
        let clock1 = Arc::clone(&clock);
        os.set(
            "clock",
            lua.create_function(move |_, ()| Ok(clock1.load(Ordering::Relaxed) as f64 / 1000.0))?,
        )?;
        let clock2 = Arc::clone(&clock);
        os.set(
            "time",
            lua.create_function(move |_, ()| {
                Ok(1700000000u64 + clock2.load(Ordering::Relaxed) / 1000)
            })?,
        )?;
        guard(lua, &os, "os", &trace)?;
        lua.globals().set("os", os)?;
        let loaders = lua.create_table()?;
        for (name, source) in &libraries {
            loaders.set(
                name.as_str(),
                lua.load(&windower_rs::lua_syntax::normalize(source).bytes)
                    .set_name(format!("@pinned-library/{name}.lua"))
                    .into_function()?,
            )?;
        }
        lua.globals().set("__replay_libraries", loaders)?;
        lua.load(include_str!("replay_support.lua"))
            .set_name("@rust-replay/explicit-mocks.lua")
            .exec()
    })();
    install.map_err(|e| e.to_string())?;
    let normalized = windower_rs::lua_syntax::normalize(source.as_bytes());
    let library_transforms = libraries
        .iter()
        .map(|(name, bytes)| {
            (
                name.clone(),
                { let n=windower_rs::lua_syntax::normalize(bytes); serde_json::json!({"literal_methods":n.literal_methods,"immediate_functions":n.immediate_functions}) },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let load_error = lua
        .load(&normalized.bytes)
        .set_name(format!("@{}", fixture.source.display()))
        .exec()
        .err()
        .map(|e| e.to_string());
    let mut steps = Vec::new();
    let mut last = 0;
    if load_error.is_none() {
        for step in fixture.steps {
            if step.at_ms < last || step.at_ms > 3600000 || step.arguments.len() > 32 {
                return Err("Invalid replay clock/arguments".into());
            }
            last = step.at_ms;
            clock.store(last, Ordering::Relaxed);
            fuel.store(200, Ordering::Relaxed);
            let poll_errors = addon
                .poll_at(last as f64 / 1000.0)
                .map_err(|e| e.to_string())?;
            let args = step
                .arguments
                .iter()
                .map(|v| value(lua, v, 0))
                .collect::<mlua::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?;
            let outcomes = addon.emit(&step.event, MultiValue::from_vec(args.clone()));
            steps.push(serde_json::json!({"event":step.event,"at_ms":step.at_ms,"poll_errors":poll_errors,
            "callbacks":outcomes.len(),"errors":outcomes.into_iter().filter_map(|(_,r)|r.err().map(|e|e.to_string())).collect::<Vec<_>>(),
            "arguments_after":args.into_iter().map(|v|snapshot(v,0)).collect::<Vec<_>>(),
            "actions":addon.take_actions().into_iter().map(|a|a.into_protocol()).collect::<Vec<_>>() }));
        }
    }
    let font_states = fonts
        .lock()
        .map_err(|_| "Replay fonts poisoned")?
        .iter()
        .map(|t| snapshot(Value::Table(t.clone()), 0))
        .collect::<Vec<_>>();
    let log = trace.lock().map_err(|_| "Replay trace poisoned")?;
    Ok(
        serde_json::json!({"source":fixture.source,"source_sha256":fixture.sha256,"ecosystem":fixture.ecosystem,
        "syntax_normalization":{"entry_literal_methods":normalized.literal_methods,"entry_immediate_functions":normalized.immediate_functions,"library_literal_methods":library_transforms},
        "libraries":fixture.libraries.iter().map(|(n,p)|(n.clone(),serde_json::json!({"source":p.source,"sha256":p.sha256}))).collect::<BTreeMap<_,_>>(),
        "load_error":load_error,"steps":steps,"trace":*log,"font_states":font_states,
        "state_source":"explicit_simulation_fixture","library_scope":"Digest-pinned original libraries where supplied; otherwise explicit ASCII/common/settings/font mocks; simulated services and semantic-only cancel intents; wc_match supports ASCII literals only",
        "game_started":false,"game_commands_sent":0,"network_packets_sent":0}),
    )
}
pub fn read_fixture(path: &Path) -> Result<Fixture, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(1048577)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1048576 {
        return Err("Replay fixture byte limit".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}
