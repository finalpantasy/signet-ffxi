//! Thread-confined file lifecycle adapter for the persistent host.
use crate::game_addons::{Host, Stats};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, time::Duration};
use windower_rs::{addon_profile::AddonProfile, commands::Backend};

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Control {
    Load { name: String },
    Reload { name: String },
    Unload { name: String },
}

#[derive(Serialize)]
pub struct Status {
    pub loaded_sources: BTreeMap<String, String>,
    pub stats: Stats,
    pub errors: Vec<String>,
    pub pending_batches: usize,
}

pub struct ProfileHost {
    profile: AddonProfile,
    host: Host,
    loaded: BTreeMap<String, String>,
}
impl ProfileHost {
    pub fn new(mut profile: AddonProfile) -> Result<Self, String> {
        profile.validate()?;
        profile.root = profile.root.canonicalize().map_err(|e| e.to_string())?;
        let host = if profile
            .addons
            .iter()
            .any(|a| a.engine == windower_rs::runtime_options::LuaEngine::LuaJit)
        {
            Host::with_provider(
                profile
                    .luajit_provider
                    .as_ref()
                    .ok_or("LuaJIT profile provider missing")?,
            )?
        } else {
            Host::new()
        };
        Ok(Self {
            profile,
            host,
            loaded: BTreeMap::new(),
        })
    }
    /// Failures are returned per addon so a bad selection cannot hide its neighbors.
    pub fn autoload(&mut self) -> Vec<(String, String)> {
        let names: Vec<_> = self.profile.addons.iter().map(|s| s.name.clone()).collect();
        names
            .into_iter()
            .filter_map(|name| {
                self.apply(Control::Load { name: name.clone() })
                    .err()
                    .map(|e| (name, e))
            })
            .collect()
    }
    pub fn apply(&mut self, control: Control) -> Result<(), String> {
        self.apply_checked(control, None)
    }
    /// Pin the exact source snapshot used for staging; never read it twice.
    pub fn apply_pinned(&mut self, control: Control, sha256: &str) -> Result<(), String> {
        self.apply_checked(control, Some(sha256))
    }
    fn apply_checked(&mut self, control: Control, expected: Option<&str>) -> Result<(), String> {
        let reload = matches!(control, Control::Reload { .. });
        match control {
            Control::Load { name } | Control::Reload { name } => {
                let source = self.profile.resolve(&name)?;
                if expected.is_some_and(|hash| source.sha256 != hash) {
                    return Err("Addon control source digest changed".into());
                }
                if reload {
                    self.host.reload_at(
                        &source.name,
                        &source.runtime_path,
                        &source.addon_path,
                        &source.text,
                    )?;
                } else {
                    self.host.load_at_engine(
                        &source.name,
                        source.ecosystem,
                        &source.runtime_path,
                        &source.addon_path,
                        &source.text,
                        source.engine,
                    )?;
                }
                self.loaded
                    .insert(source.name.to_lowercase(), source.sha256);
            }
            Control::Unload { name } => {
                self.host.unload(&name)?;
                self.loaded.remove(&name.to_lowercase());
            }
        }
        Ok(())
    }
    pub fn tick(&mut self, now: Duration, backend: &mut impl Backend) -> Result<(), String> {
        self.host.tick(now, backend)
    }
    #[cfg(all(windows, target_pointer_width = "32", feature = "game-thread-probe"))]
    pub(crate) fn reload_candidate(
        &mut self,
        name: &str,
        entry: &str,
        expected_sha256: &str,
    ) -> Result<(), String> {
        let source = self.profile.resolve_entry(name, entry)?;
        if source.sha256 != expected_sha256 {
            return Err("Prepared addon candidate digest changed".into());
        }
        self.host.reload_at(
            &source.name,
            &source.runtime_path,
            &source.addon_path,
            &source.text,
        )?;
        self.loaded
            .insert(source.name.to_lowercase(), source.sha256);
        Ok(())
    }
    pub fn status(&self) -> Status {
        Status {
            loaded_sources: self.loaded.clone(),
            stats: self.host.stats(),
            errors: self.host.errors(),
            pending_batches: self.host.pending_batches(),
        }
    }
    pub fn provider_counts(&self) -> Result<Option<(u32, u32)>, String> {
        self.host.provider_counts()
    }
    pub fn shutdown(&mut self) {
        self.host.shutdown();
        self.loaded.clear();
    }
}
