//! Fixed persistent scenario. Its backend admits only the six nonce-scoped
//! local-server command receipts, never arbitrary IPC-provided game commands.
use crate::game_addons::Host;
use std::time::{Duration, Instant};
use windower_rs::{
    commands::{Backend, Input},
    platform::Ecosystem,
    runtime_options::LabScenario,
};

pub enum Trial {
    Persistent(Session),
    Reload(crate::lab_reload::Session),
    ProfileControl(crate::profile_control::Session),
}
impl Trial {
    pub fn new(
        nonce: &str,
        scenario: LabScenario,
        files: Option<&windower_rs::runtime_options::LabFileProfile>,
        controls: Option<&windower_rs::runtime_options::LabTest>,
    ) -> Result<Self, String> {
        if let Some(lab) = controls {
            return Ok(Self::ProfileControl(crate::profile_control::Session::new(
                lab,
            )?));
        }
        if let Some(files) = files {
            return Ok(Self::Reload(crate::lab_reload::Session::new_files(
                nonce, scenario, files,
            )?));
        }
        match scenario {
            LabScenario::Persistent => Ok(Self::Persistent(Session::new(nonce)?)),
            _ => Ok(Self::Reload(crate::lab_reload::Session::new(
                nonce, scenario,
            )?)),
        }
    }
    pub fn tick(
        &mut self,
        backend: &mut impl Backend,
    ) -> Result<Option<serde_json::Value>, String> {
        match self {
            Self::Persistent(session) => session.tick(backend),
            Self::Reload(session) => session.tick(backend),
            Self::ProfileControl(session) => session.tick(),
        }
    }
}

pub struct Session {
    host: Host,
    start: Instant,
    allowed: Vec<Input>,
}
impl Session {
    pub fn new(nonce: &str) -> Result<Self, String> {
        if nonce.len() != 32
            || !nonce
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err("Invalid lab session nonce".into());
        }
        let mut host = Host::new();
        let mut allowed = Vec::new();
        for (origin, prefix, event, send) in [
            (
                Ecosystem::Windower4,
                "w",
                "prerender",
                "windower.send_command('input '..command)",
            ),
            (
                Ecosystem::Ashita4,
                "a",
                "d3d_present",
                "AshitaCore:GetChatManager():QueueCommand(1,command)",
            ),
            (
                Ecosystem::Native,
                "n",
                "frame",
                "ffxi.commands.input(command)",
            ),
        ] {
            let register = match origin {
                Ecosystem::Windower4 => "windower.register_event",
                Ecosystem::Ashita4 => {
                    "function(event,fn) ashita.events.register(event,'rust-live',fn) end"
                }
                _ => "ffxi.events.on",
            };
            let source = format!(
                r#"
                local register={register}
                local function send(command) {send} end
                local frames=0
                register('load',function()
                    coroutine.schedule(function() send('/say !rustlab {nonce} {prefix}load') end,{load_at})
                    coroutine.schedule(function() send('/say !rustlab {nonce} {prefix}delay') end,{delay_at})
                    coroutine.schedule(function() send('/echo Rust live cancelled schedule MUST NOT RUN') end,30)
                end)
                register('{event}',function()
                    frames=frames+1
                    if frames==2 then
                        coroutine.schedule(function()
                            ffxi.commands.queue('wait 30;input /echo Rust live cancelled command MUST NOT RUN')
                        end,16)
                    end
                end)
            "#,
                load_at = allowed.len() * 3,
                delay_at = allowed.len() * 3 + 3,
            );
            host.load(prefix, origin, &source)?;
            for step in ["load", "delay"] {
                allowed.push(Input {
                    origin,
                    mode: 1,
                    bytes: format!("/say !rustlab {nonce} {prefix}{step}").into_bytes(),
                });
            }
        }
        Ok(Self {
            host,
            start: Instant::now(),
            allowed,
        })
    }
    pub fn tick(
        &mut self,
        backend: &mut impl Backend,
    ) -> Result<Option<serde_json::Value>, String> {
        self.tick_at(self.start.elapsed(), backend)
    }
    fn tick_at(
        &mut self,
        now: Duration,
        backend: &mut impl Backend,
    ) -> Result<Option<serde_json::Value>, String> {
        struct Limited<'a, B> {
            backend: &'a mut B,
            allowed: &'a [Input],
        }
        impl<B: Backend> Backend for Limited<'_, B> {
            fn input(&mut self, input: &Input) -> Result<(), String> {
                if !self.allowed.contains(input) {
                    return Err("Unexpected live lab input rejected".into());
                }
                self.backend.input(input)
            }
        }
        let mut backend = Limited {
            backend,
            allowed: &self.allowed,
        };
        self.host.tick(now, &mut backend)?;
        if self.host.stats().errors != 0 {
            return Err(format!("Persistent host errors: {:?}", self.host.errors()));
        }
        if now < Duration::from_secs(18) {
            return Ok(None);
        }
        self.host.shutdown();
        let before = self.host.stats();
        self.host
            .tick(now + Duration::from_secs(36), &mut backend)?;
        let stats = self.host.stats();
        if stats.delivered != 6
            || stats.delivered != before.delivered
            || stats.loads != 3
            || stats.unloads != 3
            || stats.cancelled_batches != 3
            || stats.cancelled_schedules != 3
            || stats.errors != 0
            || stats.frames < 2
            || self.host.addon_count() != 0
            || self.host.pending_batches() != 0
        {
            return Err(format!(
                "Persistent lab lifecycle/count check failed: {stats:?}; addons={}; batches={}",
                self.host.addon_count(),
                self.host.pending_batches()
            ));
        }
        Ok(Some(
            serde_json::json!({"stats":stats,"addons_remaining":0,"pending_batches":0,
            "clock_after_shutdown_advanced_without_effects":true,"server_receipts_required":6}),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Capture(Vec<Input>);
    impl Backend for Capture {
        fn input(&mut self, input: &Input) -> Result<(), String> {
            self.0.push(input.clone());
            Ok(())
        }
    }
    #[test]
    fn persistent_scenario_runs_frames_delays_and_cancels_both_schedulers() {
        let mut session = Session::new(&"a".repeat(32)).unwrap();
        let mut backend = Capture::default();
        for ms in [0, 100, 1000, 3000, 6000, 9000, 12000, 15000, 18000] {
            let result = session
                .tick_at(Duration::from_millis(ms), &mut backend)
                .unwrap();
            assert_eq!(result.is_some(), ms == 18000);
        }
        assert_eq!(backend.0.len(), 6);
        for input in backend.0 {
            assert!(input.bytes.starts_with(b"/say !rustlab "));
        }
    }
    #[test]
    fn persistent_server_inputs_are_separated_by_three_seconds() {
        let mut session = Session::new(&"a".repeat(32)).unwrap();
        let mut backend = Capture::default();
        let mut times = Vec::new();
        for ms in (0..=18000).step_by(100) {
            let before = backend.0.len();
            session
                .tick_at(Duration::from_millis(ms), &mut backend)
                .unwrap();
            if backend.0.len() != before {
                assert_eq!(backend.0.len(), before + 1);
                times.push(ms);
            }
        }
        assert_eq!(times, vec![0, 3000, 6000, 9000, 12000, 15000]);
    }
}
