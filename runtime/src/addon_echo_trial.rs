//! Embedded, bounded Lua-to-command trial. Stage every action before enabling
//! game dispatch. No user script, addon file, chat channel or general backend.
use crate::lua::Addon;
use mlua::MultiValue;
use windower_rs::{
    commands::{self, Input, Step},
    platform::Ecosystem,
};

pub const MARKER: &[u8] = b"/echo RustAddons\0";
pub const RESPONSES: [&str; 3] = [
    "Rust Windower addon bridge verified",
    "Rust Ashita addon bridge verified",
    "Rust native addon bridge verified",
];
const FIXTURES: [(Ecosystem, &str); 3] = [
    (
        Ecosystem::Windower4,
        r#"
        windower.register_event('load', function()
            windower.send_command('input /echo Rust Windower addon bridge verified')
        end)
    "#,
    ),
    (
        Ecosystem::Ashita4,
        r#"
        ashita.events.register('load', 'rust-echo', function()
            AshitaCore:GetChatManager():QueueCommand(1, '/echo Rust Ashita addon bridge verified')
        end)
    "#,
    ),
    (
        Ecosystem::Native,
        r#"
        ffxi.events.on('load', function()
            ffxi.commands.input('/echo Rust native addon bridge verified')
        end)
    "#,
    ),
];

pub fn prepare() -> Result<Vec<Input>, String> {
    stage(&FIXTURES)
}

fn stage(fixtures: &[(Ecosystem, &str); 3]) -> Result<Vec<Input>, String> {
    let mut inputs = Vec::new();
    // All VMs finish on the staging thread. Only owned command bytes cross to
    // the game thread; no Lua state or native address is shared with Lua.
    for (index, &(origin, source)) in fixtures.iter().enumerate() {
        let addon = Addon::from_source_with_ecosystem("embedded-rust-echo", "", "", source, origin)
            .map_err(|e| e.to_string())?;
        if !addon.take_actions().is_empty() {
            return Err("Trial addon queued top-level effects".into());
        }
        let callbacks = addon.emit("load", MultiValue::new());
        if callbacks.len() != 1 || callbacks.iter().any(|(_, outcome)| outcome.is_err()) {
            return Err("Trial addon load callback failed".into());
        }
        let mut steps = Vec::new();
        for action in addon.take_actions() {
            steps.extend(commands::compile(action.into_protocol())?);
        }
        let expected = Input {
            origin,
            mode: 1,
            bytes: format!("/echo {}", RESPONSES[index]).into_bytes(),
        };
        if steps != vec![Step::Input(expected.clone())] {
            return Err("Trial addon produced an unexpected action".into());
        }
        if addon.scheduled_count().map_err(|e| e.to_string())? != 0
            || !addon.emit("unload", MultiValue::new()).is_empty()
            || !addon.take_actions().is_empty()
        {
            return Err("Trial addon left deferred work".into());
        }
        inputs.push(expected);
    }
    Ok(inputs)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn three_real_lua_dialects_stage_exact_local_inputs_in_order() {
        let inputs = prepare().unwrap();
        assert_eq!(inputs.len(), 3);
        for (i, input) in inputs.iter().enumerate() {
            assert_eq!(input.origin, FIXTURES[i].0);
            assert_eq!(input.mode, 1);
            assert_eq!(input.bytes, format!("/echo {}", RESPONSES[i]).as_bytes());
        }
    }
    #[test]
    fn failed_callback_or_unexpected_effect_discards_the_entire_staged_batch() {
        for source in [
            "windower.register_event('load', function() error('stop') end)",
            "windower.register_event('load', function() windower.send_command('input /say unexpected') end)",
            "windower.send_command('input /echo early')",
            "windower.register_event('load', function() coroutine.schedule(function() end, 5) end)",
        ] {
            let mut fixtures = FIXTURES;
            fixtures[2] = (Ecosystem::Native, source);
            assert!(stage(&fixtures).is_err());
        }
    }
}
