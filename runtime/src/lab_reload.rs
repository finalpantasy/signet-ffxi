//! Fixed live reload fixtures. Only six nonce-scoped server commands are admitted.
use crate::game_addons::Host;
use crate::profile_addons::{Control, ProfileHost};
use std::time::{Duration, Instant};
use windower_rs::{
    commands::{Backend, Input},
    platform::Ecosystem,
    runtime_options::{LabFileProfile, LabScenario, LuaEngine},
};

enum ReloadHost {
    Embedded(Host),
    Files {
        host: ProfileHost,
        fixture: LabFileProfile,
        snapshots: Vec<serde_json::Value>,
    },
}
impl ReloadHost {
    fn tick(&mut self, now: Duration, backend: &mut impl Backend) -> Result<(), String> {
        match self {
            Self::Embedded(h) => h.tick(now, backend),
            Self::Files { host, .. } => host.tick(now, backend),
        }
    }
    fn stats(&self) -> crate::game_addons::Stats {
        match self {
            Self::Embedded(h) => h.stats(),
            Self::Files { host, .. } => host.status().stats,
        }
    }
    fn errors(&self) -> Vec<String> {
        match self {
            Self::Embedded(h) => h.errors(),
            Self::Files { host, .. } => host.status().errors,
        }
    }
    fn counts(&self) -> (usize, usize) {
        match self {
            Self::Embedded(h) => (h.addon_count(), h.pending_batches()),
            Self::Files { host, .. } => {
                let s = host.status();
                (s.loaded_sources.len(), s.pending_batches)
            }
        }
    }
    fn snapshot(&mut self, phase: &str) {
        if let Self::Files {
            host, snapshots, ..
        } = self
        {
            snapshots.push(serde_json::json!({"phase":phase,"status":host.status()}));
        }
    }
    fn reload(&mut self, source: &str, leaf: &str) -> Result<(), String> {
        match self {
            Self::Embedded(h) => h.reload("selected", source),
            Self::Files { host, fixture, .. } => {
                let hash = fixture
                    .sources
                    .get(&format!("selected/{leaf}"))
                    .ok_or("Missing pinned candidate")?;
                host.reload_candidate("selected", leaf, hash)
            }
        }
    }
    fn unload_selected(&mut self) -> Result<(), String> {
        if let Self::Files { host, .. } = self {
            host.apply(Control::Unload {
                name: "selected".into(),
            })?;
        }
        Ok(())
    }
    fn shutdown(&mut self) {
        match self {
            Self::Embedded(h) => h.shutdown(),
            Self::Files { host, .. } => host.shutdown(),
        }
    }
    fn file_evidence(&self) -> Option<serde_json::Value> {
        match self {
            Self::Embedded(_) => None,
            Self::Files {
                fixture, snapshots, ..
            } => Some(
                serde_json::json!({"profile_sha256":fixture.sha256,"sources_sha256":fixture.sources,"snapshots":snapshots,"typed_selected_unload":true}),
            ),
        }
    }
}

fn dialect(origin: Ecosystem) -> (&'static str, &'static str) {
    match origin {
        Ecosystem::Windower4 => (
            "windower.register_event",
            "windower.send_command('input '..command)",
        ),
        Ecosystem::Ashita4 => (
            "function(event,fn) ashita.events.register(event,'reload-fixture',fn) end",
            "AshitaCore:GetChatManager():QueueCommand(1,command)",
        ),
        _ => ("ffxi.events.on", "ffxi.commands.input(command)"),
    }
}

pub struct Session {
    host: ReloadHost,
    start: Instant,
    allowed: Vec<Input>,
    scenario: LabScenario,
    replacement: String,
    rejected_source: String,
    rejected: Vec<String>,
    replaced: bool,
    unloaded: bool,
}
impl Session {
    pub fn new(nonce: &str, scenario: LabScenario) -> Result<Self, String> {
        Self::prepare(nonce, scenario, None)
    }
    pub fn new_files(
        nonce: &str,
        scenario: LabScenario,
        files: &LabFileProfile,
    ) -> Result<Self, String> {
        Self::prepare(nonce, scenario, Some(files))
    }
    fn prepare(
        nonce: &str,
        scenario: LabScenario,
        files: Option<&LabFileProfile>,
    ) -> Result<Self, String> {
        if nonce.len() != 32
            || !nonce
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err("Invalid reload lab nonce".into());
        }
        let (selected, neighbor) = match scenario {
            LabScenario::ReloadWindower => (Ecosystem::Windower4, Ecosystem::Ashita4),
            LabScenario::ReloadAshita => (Ecosystem::Ashita4, Ecosystem::Native),
            LabScenario::ReloadNative => (Ecosystem::Native, Ecosystem::Windower4),
            _ => return Err("Reload lab requires an explicit reload scenario".into()),
        };
        let make = |origin, prefix: &str, initial, first, second, pending| {
            let (register, send) = dialect(origin);
            format!(
                r#"
                local register={register}
                local function send(command) {send} end
                local count={initial}
                local function witness()
                    count=count+1
                    send('/say !rustlab {nonce} reload_{prefix}'..count)
                end
                register('load',function()
                    coroutine.schedule(witness,{first})
                    coroutine.schedule(witness,{second})
                    coroutine.schedule(witness,30)
                    coroutine.schedule(function()
                        ffxi.commands.queue('wait 30;input /echo Rust retired reload batch MUST NOT RUN')
                    end,{pending})
                end)
                register('unload',function()
                    send('/echo Rust unload effects MUST NOT RUN')
                end)
            "#
            )
        };
        let mut host = if let Some(files) = files {
            files.validate()?;
            let profile = windower_rs::addon_profile::AddonProfile::read_verified(
                &files.path,
                &files.sha256,
            )?;
            if profile.root.canonicalize().map_err(|e| e.to_string())?
                != files
                    .path
                    .parent()
                    .ok_or("Missing profile parent")?
                    .canonicalize()
                    .map_err(|e| e.to_string())?
                || profile.addons.len() != 2
                || profile.addons[0].name != "selected"
                || profile.addons[0].ecosystem != selected
                || profile.addons[1].name != "neighbor"
                || profile.addons[1].ecosystem != neighbor
                || profile.addons.iter().any(|s| s.engine != LuaEngine::Lua51)
            {
                return Err(
                    "Prepared file profile selection/root differs from the fixed lab scenario"
                        .into(),
                );
            }
            for (name, expected) in &files.sources {
                let (addon, leaf) = name.split_once('/').ok_or("Invalid fixture source")?;
                if profile.resolve_entry(addon, leaf)?.sha256 != *expected {
                    return Err("Prepared fixture source digest changed".into());
                }
            }
            let mut host = ProfileHost::new(profile)?;
            let errors = host.autoload();
            if !errors.is_empty() {
                return Err(format!("Prepared file autoload failed: {errors:?}"));
            }
            ReloadHost::Files {
                host,
                fixture: files.clone(),
                snapshots: Vec::new(),
            }
        } else {
            let mut host = Host::new();
            host.load("selected", selected, &make(selected, "old", 0, 0, 6, 7))?;
            host.load(
                "neighbor",
                neighbor,
                &make(neighbor, "neighbor", 0, 3, 12, 16),
            )?;
            ReloadHost::Embedded(host)
        };
        host.snapshot("autoload");
        let replacement = make(selected, "new", 100, 1, 7, 8);
        let (register, send) = dialect(selected);
        let rejected_source = format!(
            "local register={register}; local function send(command) {send} end; register('load',function() send('/echo Rust rejected reload MUST NOT RUN');error('expected reload load failure') end)"
        );
        let allowed = [
            (selected, "reload_old1"),
            (neighbor, "reload_neighbor1"),
            (selected, "reload_old2"),
            (selected, "reload_new101"),
            (neighbor, "reload_neighbor2"),
            (selected, "reload_new102"),
        ]
        .into_iter()
        .map(|(origin, step)| Input {
            origin,
            mode: 1,
            bytes: format!("/say !rustlab {nonce} {step}").into_bytes(),
        })
        .collect();
        Ok(Self {
            host,
            start: Instant::now(),
            allowed,
            scenario,
            replacement,
            rejected_source,
            rejected: Vec::new(),
            replaced: false,
            unloaded: false,
        })
    }
    pub fn tick(
        &mut self,
        backend: &mut impl Backend,
    ) -> Result<Option<serde_json::Value>, String> {
        self.tick_at(self.start.elapsed(), backend)
    }
    pub(crate) fn tick_at(
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
                    return Err("Unexpected reload input rejected".into());
                }
                self.backend.input(input)
            }
        }
        let mut limited = Limited {
            backend,
            allowed: &self.allowed,
        };
        self.host.tick(now, &mut limited)?;
        if self.rejected.is_empty() && now >= Duration::from_secs(4) {
            if self.host.stats().delivered != 2 {
                return Err("Missed the bounded failed-reload window".into());
            }
            for (source, leaf, expected) in [
                ("syntax ???", "syntax.lua", "syntax error"),
                (
                    &self.rejected_source,
                    "load-error.lua",
                    "expected reload load failure",
                ),
            ] {
                let error = self
                    .host
                    .reload(source, leaf)
                    .err()
                    .ok_or("An intentionally broken replacement was accepted")?;
                if !error.contains(expected) {
                    return Err(format!(
                        "Replacement failed outside its expected fixture error: {error}"
                    ));
                }
                self.rejected.push(error);
                self.host.snapshot(leaf);
            }
        }
        if !self.replaced && now >= Duration::from_secs(8) {
            if self.rejected.len() != 2 || self.host.stats().delivered != 3 {
                return Err("Old addon did not survive both rejected reloads".into());
            }
            self.host.reload(&self.replacement, "replacement.lua")?;
            self.host.snapshot("replacement");
            self.replaced = true;
        }
        if self.host.stats().errors != 0 {
            return Err(format!("Reload host errors: {:?}", self.host.errors()));
        }
        if !self.unloaded
            && now >= Duration::from_secs(17)
            && matches!(self.host, ReloadHost::Files { .. })
        {
            if self.host.stats().delivered != 6 {
                return Err("File unload requested before all six deliveries".into());
            }
            self.host.unload_selected()?;
            self.host.snapshot("selected_unloaded");
            if self.host.counts().0 != 1 {
                return Err("File unload did not preserve the neighbor".into());
            }
            self.unloaded = true;
        }
        if now < Duration::from_secs(18) {
            return Ok(None);
        }
        self.host.shutdown();
        self.host.snapshot("shutdown");
        let before = self.host.stats();
        self.host
            .tick(now + Duration::from_secs(60), &mut limited)?;
        let stats = self.host.stats();
        if stats.delivered != 6
            || stats.delivered != before.delivered
            || stats.loads != 3
            || stats.unloads != 3
            || stats.reloads != 1
            || stats.cancelled_batches != 3
            || stats.cancelled_schedules != 3
            || stats.errors != 0
            || self.host.counts() != (0, 0)
        {
            return Err(format!(
                "Reload lab lifecycle/count check failed: {stats:?}"
            ));
        }
        Ok(Some(serde_json::json!({
            "scenario":self.scenario,"stats":stats,"rejected_reloads":self.rejected.len(),
            "rejected_errors":self.rejected,"successful_reloads":1,
            "old_state_witnesses":[1,2],"replacement_state_witnesses":[101,102],
            "neighbor_state_witnesses":[1,2],"server_receipts_required":6,
            "addons_remaining":0,"pending_batches":0,
            "clock_after_shutdown_advanced_without_effects":true
            ,"file_profile":self.host.file_evidence()
        })))
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
    fn three_dialects_preserve_old_and_neighbor_state_and_cancel_retired_work() {
        for scenario in [
            LabScenario::ReloadWindower,
            LabScenario::ReloadAshita,
            LabScenario::ReloadNative,
        ] {
            let mut session = Session::new(&"a".repeat(32), scenario).unwrap();
            let mut capture = Capture::default();
            let mut times = Vec::new();
            for ms in (0..=18000).step_by(100) {
                let previous = capture.0.len();
                let result = session
                    .tick_at(Duration::from_millis(ms), &mut capture)
                    .unwrap();
                assert_eq!(result.is_some(), ms == 18000);
                if capture.0.len() != previous {
                    assert_eq!(capture.0.len(), previous + 1);
                    times.push(ms);
                }
                if let Some(report) = result {
                    assert_eq!(report["rejected_reloads"], 2);
                    assert_eq!(report["successful_reloads"], 1);
                }
            }
            assert_eq!(capture.0, session.allowed);
            assert_eq!(times, vec![0, 3000, 6000, 9000, 12000, 15000]);
        }
    }
    #[test]
    fn reload_rejects_unknown_scenario_and_bad_nonce_before_loading() {
        assert!(Session::new(&"a".repeat(32), LabScenario::Persistent).is_err());
        assert!(Session::new(&"A".repeat(32), LabScenario::ReloadWindower).is_err());
    }
}
