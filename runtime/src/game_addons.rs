//! Persistent, single-thread addon lifecycle and owner-scoped game commands.
//! No global bookkeeping lock is held across Lua callbacks or destruction.
use crate::persistent_vm::Vm;
use std::{collections::VecDeque, marker::PhantomData, rc::Rc, time::Duration};
use windower_rs::{
    commands::{Backend, Dispatcher},
    platform::Ecosystem,
};

struct Entry {
    owner: u64,
    name: String,
    addon: Vm,
    epoch: Duration,
}
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Stats {
    pub frames: u64,
    pub callbacks: u64,
    pub errors: u64,
    pub delivered: u64,
    pub loads: u64,
    pub unloads: u64,
    pub reloads: u64,
    pub cancelled_batches: u64,
    pub cancelled_schedules: u64,
}
pub struct Host {
    entries: Vec<Entry>,
    commands: Dispatcher,
    next_owner: u64,
    now: Duration,
    stopped: bool,
    stats: Stats,
    errors: VecDeque<String>,
    _thread_confined: PhantomData<Rc<()>>,
    #[cfg(all(windows, target_pointer_width = "32"))]
    provider: Option<std::sync::Arc<crate::vm_client::Provider>>,
}
impl Default for Host {
    fn default() -> Self {
        Self::new()
    }
}
impl Host {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            commands: Dispatcher::default(),
            next_owner: 1,
            now: Duration::ZERO,
            stopped: false,
            stats: Stats::default(),
            errors: VecDeque::new(),
            _thread_confined: PhantomData,
            #[cfg(all(windows, target_pointer_width = "32"))]
            provider: None,
        }
    }
    pub fn with_provider(
        selection: &windower_rs::runtime_options::NativeLibrary,
    ) -> Result<Self, String> {
        selection.verify()?;
        #[cfg(all(windows, target_pointer_width = "32"))]
        {
            // Caller-selected pinned provider is trusted native code, like the existing VM workbench.
            let provider = unsafe { crate::vm_client::Provider::open(&selection.path) }?;
            let mut host = Self::new();
            host.provider = Some(std::sync::Arc::new(provider));
            Ok(host)
        }
        #[cfg(not(all(windows, target_pointer_width = "32")))]
        Err("LuaJIT file hosting requires Windows x86".into())
    }
    pub fn provider_counts(&self) -> Result<Option<(u32, u32)>, String> {
        #[cfg(all(windows, target_pointer_width = "32"))]
        {
            self.provider
                .as_ref()
                .map(|p| p.resource_counts())
                .transpose()
        }
        #[cfg(not(all(windows, target_pointer_width = "32")))]
        {
            Ok(None)
        }
    }
    pub fn stats(&self) -> Stats {
        self.stats.clone()
    }
    pub fn errors(&self) -> Vec<String> {
        self.errors.iter().cloned().collect()
    }
    pub fn addon_count(&self) -> usize {
        self.entries.len()
    }
    pub fn pending_batches(&self) -> usize {
        self.commands.pending_batches()
    }
    fn error(&mut self, message: String) {
        self.stats.errors = self.stats.errors.saturating_add(1);
        if self.errors.len() == 128 {
            self.errors.pop_front();
        }
        self.errors.push_back(message);
    }
    fn stage(
        &mut self,
        name: &str,
        ecosystem: Ecosystem,
        runtime_path: &str,
        addon_path: &str,
        source: &str,
        engine: windower_rs::runtime_options::LuaEngine,
    ) -> Result<Entry, String> {
        if self.stopped {
            return Err("Addon host is stopped".into());
        }
        if name.is_empty() || name.len() > 256 || name.contains(['/', '\\', ':', '\0']) {
            return Err("Invalid addon name".into());
        }
        let addon = match engine {
            windower_rs::runtime_options::LuaEngine::Lua51 => {
                Vm::local(name, runtime_path, addon_path, source, ecosystem)?
            }
            windower_rs::runtime_options::LuaEngine::LuaJit => {
                #[cfg(all(windows, target_pointer_width = "32"))]
                {
                    Vm::foreign(
                        self.provider
                            .clone()
                            .ok_or("No LuaJIT file provider selected")?,
                        name,
                        runtime_path,
                        addon_path,
                        source,
                        ecosystem,
                    )?
                }
                #[cfg(not(all(windows, target_pointer_width = "32")))]
                {
                    return Err("LuaJIT file hosting requires Windows x86".into());
                }
            }
        };
        let results = addon.emit("load");
        for (_, result) in &results {
            if let Err(error) = result {
                return Err(error.to_string());
            }
        }
        let owner = self.next_owner;
        self.next_owner = owner.checked_add(1).ok_or("Addon generation limit")?;
        self.stats.callbacks += results.len() as u64;
        Ok(Entry {
            owner,
            name: name.into(),
            addon,
            epoch: self.now,
        })
    }
    fn collect(&mut self, entry: &Entry) {
        for action in entry.addon.take_actions() {
            if let Err(message) = self.commands.enqueue(entry.owner, action, self.now) {
                self.error(format!("{}: {message}", entry.name));
            }
        }
    }
    pub fn load(&mut self, name: &str, ecosystem: Ecosystem, source: &str) -> Result<(), String> {
        self.load_at(name, ecosystem, "", "", source)
    }
    pub fn load_at(
        &mut self,
        name: &str,
        ecosystem: Ecosystem,
        runtime_path: &str,
        addon_path: &str,
        source: &str,
    ) -> Result<(), String> {
        self.load_at_engine(
            name,
            ecosystem,
            runtime_path,
            addon_path,
            source,
            windower_rs::runtime_options::LuaEngine::Lua51,
        )
    }
    pub fn load_at_engine(
        &mut self,
        name: &str,
        ecosystem: Ecosystem,
        runtime_path: &str,
        addon_path: &str,
        source: &str,
        engine: windower_rs::runtime_options::LuaEngine,
    ) -> Result<(), String> {
        if self
            .entries
            .iter()
            .any(|e| e.name.eq_ignore_ascii_case(name))
        {
            return Err("Addon already loaded".into());
        }
        if self.entries.len() >= 256 {
            return Err("Addon capacity reached".into());
        }
        let entry = self.stage(name, ecosystem, runtime_path, addon_path, source, engine)?;
        self.collect(&entry);
        self.entries.push(entry);
        self.stats.loads += 1;
        Ok(())
    }
    fn retire(&mut self, entry: Entry) {
        // Remove every old-generation command before unload executes. Unload
        // callbacks may run cleanup but cannot resurrect game effects.
        let before = self.commands.pending_batches();
        self.commands.cancel(entry.owner);
        self.stats.cancelled_batches += (before - self.commands.pending_batches()) as u64;
        self.stats.cancelled_schedules += entry.addon.scheduled_count().unwrap_or(0) as u64;
        let results = entry.addon.emit("unload");
        self.stats.callbacks += results.len() as u64;
        for (_, result) in results {
            if let Err(message) = result {
                self.error(format!("{}: {message}", entry.name));
            }
        }
        entry.addon.take_actions();
        self.stats.unloads += 1;
        drop(entry);
    }
    pub fn unload(&mut self, name: &str) -> Result<(), String> {
        let index = self
            .entries
            .iter()
            .position(|e| e.name.eq_ignore_ascii_case(name))
            .ok_or("Addon not loaded")?;
        let entry = self.entries.remove(index);
        self.retire(entry);
        Ok(())
    }
    pub fn reload(&mut self, name: &str, source: &str) -> Result<(), String> {
        self.reload_at(name, "", "", source)
    }
    pub fn reload_at(
        &mut self,
        name: &str,
        runtime_path: &str,
        addon_path: &str,
        source: &str,
    ) -> Result<(), String> {
        let index = self
            .entries
            .iter()
            .position(|e| e.name.eq_ignore_ascii_case(name))
            .ok_or("Addon not loaded")?;
        let ecosystem = self.entries[index].addon.ecosystem();
        // Failed source/load leaves the previous VM, schedules and commands intact.
        let engine = self.entries[index].addon.engine();
        let replacement = self.stage(name, ecosystem, runtime_path, addon_path, source, engine)?;
        let previous = std::mem::replace(&mut self.entries[index], replacement);
        self.retire(previous);
        let entry = self.entries.remove(index);
        self.collect(&entry);
        self.entries.insert(index, entry);
        self.stats.loads += 1;
        self.stats.reloads += 1;
        Ok(())
    }
    pub fn tick(&mut self, now: Duration, backend: &mut impl Backend) -> Result<(), String> {
        if now < self.now {
            return Err("Addon clock moved backward".into());
        }
        self.now = now;
        if !self.stopped {
            self.stats.frames = self.stats.frames.saturating_add(1);
            // Moving owned entries out avoids a host lock or aliasing mutable
            // host state across any arbitrary callback. No callbacks receive Host.
            let entries = std::mem::take(&mut self.entries);
            for entry in &entries {
                match entry
                    .addon
                    .poll_at(now.saturating_sub(entry.epoch).as_secs_f64())
                {
                    Ok(errors) => {
                        for message in errors {
                            self.error(format!("{}: {message}", entry.name));
                        }
                    }
                    Err(message) => self.error(format!("{}: {message}", entry.name)),
                }
                let results = entry
                    .addon
                    .emit_frame(self.stats.frames.min(u32::MAX as u64) as u32);
                self.stats.callbacks += results.len() as u64;
                for (_, result) in results {
                    if let Err(message) = result {
                        self.error(format!("{}: {message}", entry.name));
                    }
                }
                self.collect(entry);
            }
            self.entries = entries;
        }
        let report = self.commands.pump(now, backend)?;
        self.stats.delivered += report.delivered as u64;
        for (owner, error) in report.failures {
            self.error(format!("addon generation {owner}: {error}"));
        }
        Ok(())
    }
    pub fn shutdown(&mut self) {
        self.stopped = true;
        for entry in std::mem::take(&mut self.entries) {
            self.retire(entry);
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windower_rs::commands::Input;
    #[derive(Default)]
    struct Capture(Vec<Vec<u8>>);
    impl Backend for Capture {
        fn input(&mut self, input: &Input) -> Result<(), String> {
            self.0.push(input.bytes.clone());
            Ok(())
        }
    }
    fn time(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }
    #[test]
    fn persistent_frames_schedules_and_owner_cancellation() {
        let mut host = Host::new();
        let mut backend = Capture::default();
        host.load("windower",Ecosystem::Windower4,r#"
            windower.register_event('load',function()
                windower.send_command('input /echo load;wait 0.25;input /echo delayed')
                windower.send_command('wait 5;input /echo stale')
                coroutine.schedule(function() windower.send_command('input /echo stale-lua') end,5)
            end)
            local count=0
            windower.register_event('prerender',function() count=count+1; if count==2 then windower.send_command('input /echo frame') end end)
            windower.register_event('unload',function() windower.send_command('input /echo stale-unload') end)
        "#).unwrap();
        host.tick(time(0), &mut backend).unwrap();
        host.tick(time(100), &mut backend).unwrap();
        assert_eq!(backend.0, vec![b"/echo load".to_vec()]);
        host.tick(time(250), &mut backend).unwrap();
        assert_eq!(
            backend.0,
            vec![b"/echo load".to_vec(), b"/echo delayed".to_vec()]
        );
        // FIFO per owner also keeps the frame action behind its earlier wait.
        host.unload("WINDOWer").unwrap();
        host.tick(time(6000), &mut backend).unwrap();
        assert_eq!(backend.0.len(), 2);
        assert_eq!(host.pending_batches(), 0);
        assert_eq!(host.stats().cancelled_schedules, 1);
        assert_eq!(host.stats().unloads, 1);
        assert!(host.stats().cancelled_batches >= 1);
        host.shutdown();
        host.shutdown();
        assert_eq!(host.stats().unloads, 1);
        assert_eq!(host.stats().errors, 0);
    }
    #[test]
    fn reload_rolls_back_failure_and_cancels_only_old_generation() {
        let mut host = Host::new();
        let mut backend = Capture::default();
        let old = "windower.register_event('load',function() coroutine.schedule(function() windower.send_command('input /echo old') end,1) end)";
        host.load("selected", Ecosystem::Windower4, old).unwrap();
        host.load("neighbor",Ecosystem::Native,"local n=0;ffxi.events.on('frame',function() n=n+1;if n==3 then ffxi.commands.input('/echo neighbor') end end)").unwrap();
        assert!(host.reload("selected", "syntax ???").is_err());
        host.tick(time(0), &mut backend).unwrap();
        host.tick(time(1000), &mut backend).unwrap();
        assert_eq!(backend.0, vec![b"/echo old".to_vec()]);
        host.reload("selected","windower.register_event('load',function() windower.send_command('wait 5;input /echo retired') end)").unwrap();
        host.tick(time(1100), &mut backend).unwrap();
        host.reload("selected","windower.register_event('load',function() windower.send_command('input /echo replacement') end)").unwrap();
        host.tick(time(7000), &mut backend).unwrap();
        assert_eq!(
            backend.0,
            vec![
                b"/echo old".to_vec(),
                b"/echo neighbor".to_vec(),
                b"/echo replacement".to_vec()
            ]
        );
        assert_eq!(host.stats().reloads, 2);
        assert_eq!(host.stats().cancelled_batches, 1);
    }
    #[test]
    fn callback_errors_do_not_stop_neighbors_and_backward_clock_has_no_effect() {
        let mut host = Host::new();
        let mut backend = Capture::default();
        host.load(
            "bad",
            Ecosystem::Ashita4,
            "ashita.events.register('d3d_present','bad',function() error('fixture failure') end)",
        )
        .unwrap();
        host.load("good",Ecosystem::Native,"local n=0;ffxi.events.on('frame',function() n=n+1;ffxi.commands.input('/echo '..n) end)").unwrap();
        host.tick(time(1), &mut backend).unwrap();
        assert!(host.tick(time(0), &mut backend).is_err());
        host.tick(time(2), &mut backend).unwrap();
        assert_eq!(backend.0, vec![b"/echo 1".to_vec(), b"/echo 2".to_vec()]);
        assert_eq!(host.stats().errors, 2);
        assert_eq!(host.stats().frames, 2);
        host.shutdown();
        host.tick(time(9999), &mut backend).unwrap();
        assert_eq!(backend.0.len(), 2);
        assert!(host.load("late", Ecosystem::Native, "").is_err());
    }
}
