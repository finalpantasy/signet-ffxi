//! Our versioned configuration lives outside the legacy 42,180-byte packet.
//! A child-specific mapping prevents changing another launcher's environment.
use crate::platform::Ecosystem;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

pub const OPTIONS_BYTES: usize = 65536;
const MAGIC: &[u8; 8] = b"WRSCFG01";

pub fn mapping_name(process_id: u32) -> String {
    format!("WindowerRust.{process_id}.options")
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GraphicsBackend {
    #[default]
    System,
    External {
        path: PathBuf,
        sha256: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeOptions {
    pub schema_version: u32,
    pub graphics: GraphicsBackend,
    #[serde(default)]
    pub addons: Vec<AddonSpec>,
    #[serde(default)]
    pub luajit_provider: Option<NativeLibrary>,
    #[serde(default)]
    pub client_diagnostic: Option<PathBuf>,
    #[serde(default)]
    pub game_thread_probe: Option<GameThreadProbe>,
    #[serde(default)]
    pub virtual_display: bool,
    /// Cache DirectInput controller discovery off the caller thread. Set false
    /// only to compare the original enumeration path during diagnostics.
    #[serde(default = "controller_discovery_default")]
    pub controller_discovery: bool,
}
/// Deliberately fixed local echo trial; never an arbitrary command interface.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameThreadProbe {
    pub ready_report: PathBuf,
    pub result_report: PathBuf,
    pub game_sha256: String,
    /// Run only the three embedded local-echo fixtures; never disk addons.
    #[serde(default)]
    pub addon_echo_trial: bool,
    #[serde(default)]
    pub lab_test: Option<LabTest>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabTest {
    pub nonce: String,
    pub request: PathBuf,
    #[serde(default)]
    pub scenario: LabScenario,
    #[serde(default)]
    pub file_profile: Option<LabFileProfile>,
    #[serde(default)]
    pub profile_control: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabFileProfile {
    pub path: PathBuf,
    pub sha256: String,
    pub sources: std::collections::BTreeMap<String, String>,
}
impl LabFileProfile {
    pub fn validate(&self) -> Result<(), String> {
        NativeLibrary {
            path: self.path.clone(),
            sha256: self.sha256.clone(),
        }
        .validate()?;
        let expected = [
            "neighbor/helper.lua",
            "neighbor/neighbor.lua",
            "selected/helper.lua",
            "selected/load-error.lua",
            "selected/replacement.lua",
            "selected/selected.lua",
            "selected/syntax.lua",
        ];
        if self.sources.keys().map(String::as_str).collect::<Vec<_>>() != expected
            || self.sources.values().any(|h| {
                h.len() != 64
                    || !h
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            })
        {
            return Err("File lab requires exactly seven pinned fixture sources".into());
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabScenario {
    #[default]
    Persistent,
    ReloadWindower,
    ReloadAshita,
    ReloadNative,
}
impl LabScenario {
    pub fn operation(self) -> &'static str {
        match self {
            Self::Persistent => "run_persistent",
            Self::ReloadWindower => "run_reload_windower",
            Self::ReloadAshita => "run_reload_ashita",
            Self::ReloadNative => "run_reload_native",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeLibrary {
    pub path: PathBuf,
    pub sha256: String,
}
impl NativeLibrary {
    pub fn validate(&self) -> Result<(), String> {
        if !self.path.is_absolute()
            || self.sha256.len() != 64
            || !self.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("Native provider requires an absolute path and SHA-256 digest".into());
        }
        Ok(())
    }
    pub fn verify(&self) -> Result<(), String> {
        RuntimeOptions {
            graphics: GraphicsBackend::External {
                path: self.path.clone(),
                sha256: self.sha256.clone(),
            },
            ..RuntimeOptions::default()
        }
        .verify_external()
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LuaEngine {
    #[default]
    Lua51,
    LuaJit,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddonSpec {
    pub name: String,
    pub ecosystem: Ecosystem,
    #[serde(default)]
    pub engine: LuaEngine,
}
impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            schema_version: 1,
            graphics: GraphicsBackend::System,
            addons: Vec::new(),
            luajit_provider: None,
            client_diagnostic: None,
            game_thread_probe: None,
            virtual_display: false,
            controller_discovery: true,
        }
    }
}
fn controller_discovery_default() -> bool {
    true
}
impl RuntimeOptions {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("Unsupported runtime options schema".into());
        }
        if let Some(path) = &self.client_diagnostic {
            if !path.is_absolute() {
                return Err("Client diagnostic report requires an absolute path".into());
            }
            if !self.addons.is_empty() || self.luajit_provider.is_some() {
                return Err("First client diagnostic does not load addons or VM providers".into());
            }
        }
        if let Some(probe) = &self.game_thread_probe
            && (self.client_diagnostic.is_none()
                || !matches!(self.graphics, GraphicsBackend::System)
                || !probe.ready_report.is_absolute()
                || !probe.result_report.is_absolute()
                || probe.ready_report == probe.result_report
                || self.client_diagnostic.as_ref() == Some(&probe.ready_report)
                || self.client_diagnostic.as_ref() == Some(&probe.result_report)
                || probe.game_sha256.len() != 64
                || !probe.game_sha256.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err("Game thread probe requires distinct absolute reports, a game digest and the system diagnostic backend".into());
        }
        if let Some(probe) = &self.game_thread_probe
            && let Some(lab) = &probe.lab_test
            && (probe.addon_echo_trial
                || lab.nonce.len() != 32
                || !lab
                    .nonce
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                || !lab.request.is_absolute()
                || lab.request.parent() != probe.ready_report.parent()
                || [&probe.ready_report, &probe.result_report].contains(&&lab.request)
                || self.client_diagnostic.as_ref() == Some(&lab.request))
        {
            return Err("Lab test requires its own request path and lowercase nonce, without the one-shot addon fixture".into());
        }
        if self.addons.len() > 256 {
            return Err("Too many addon selections".into());
        }
        if let Some(lab) = self
            .game_thread_probe
            .as_ref()
            .and_then(|p| p.lab_test.as_ref())
            && lab.profile_control
            && lab.file_profile.is_none()
        {
            return Err("Live profile controls require an explicit prepared file profile".into());
        }
        if let Some(lab) = self
            .game_thread_probe
            .as_ref()
            .and_then(|p| p.lab_test.as_ref())
            && let Some(files) = &lab.file_profile
        {
            files.validate()?;
            if lab.scenario == LabScenario::Persistent
                || files.path
                    != lab
                        .request
                        .parent()
                        .ok_or("Missing lab directory")?
                        .join("runtime-root/lab-profile.json")
            {
                return Err("File profile trial requires a reload scenario and its private prepared profile path".into());
            }
        }
        let mut names = std::collections::BTreeSet::new();
        if let Some(provider) = &self.luajit_provider {
            provider.validate()?;
        }
        for addon in &self.addons {
            if addon.engine == LuaEngine::LuaJit && self.luajit_provider.is_none() {
                return Err("LuaJIT addon requires an explicit VM provider".into());
            }
            if addon.name.is_empty()
                || addon.name == "."
                || addon.name == ".."
                || addon.name.contains(['/', '\\', ':', '\0'])
            {
                return Err("Invalid addon directory name".into());
            }
            if !names.insert(addon.name.to_lowercase()) {
                return Err("Conflicting addon selections".into());
            }
            if addon.ecosystem == Ecosystem::Windower5 {
                return Err("Windower 5 package hosting is not implemented yet".into());
            }
        }
        if let GraphicsBackend::External { path, sha256 } = &self.graphics {
            if !path.is_absolute() || path.as_os_str().is_empty() {
                return Err("Graphics backend requires an absolute DLL path".into());
            }
            if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err("Graphics backend requires a SHA-256 digest".into());
            }
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let json = serde_json::to_vec(self).map_err(|error| error.to_string())?;
        if json.len() > OPTIONS_BYTES - 12 {
            return Err("Runtime options exceed mapping size".into());
        }
        let mut bytes = Vec::with_capacity(12 + json.len());
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
        bytes.extend(json);
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.get(..8) != Some(MAGIC) {
            return Err("Invalid runtime options magic".into());
        }
        let length = u32::from_le_bytes(
            bytes
                .get(8..12)
                .ok_or("Truncated options header")?
                .try_into()
                .map_err(|_| "Invalid options length")?,
        ) as usize;
        if length > OPTIONS_BYTES - 12 {
            return Err("Runtime options exceed mapping size".into());
        }
        let json = bytes
            .get(12..12 + length)
            .ok_or("Truncated runtime options")?;
        let result: Self = serde_json::from_slice(json).map_err(|error| error.to_string())?;
        result.validate()?;
        Ok(result)
    }
    pub fn external(path: PathBuf) -> Result<Self, String> {
        let path = path.canonicalize().map_err(|error| error.to_string())?;
        let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
        let result = Self {
            graphics: GraphicsBackend::External {
                path,
                sha256: format!("{:x}", Sha256::digest(bytes)),
            },
            ..Self::default()
        };
        result.validate()?;
        Ok(result)
    }
    /// Validate the selected bytes before loading native code. This is an
    /// identity check, not an assertion of publisher authenticity.
    pub fn verify_external(&self) -> Result<(), String> {
        self.validate()?;
        if let Some(provider) = &self.luajit_provider {
            provider.verify()?;
        }
        if let GraphicsBackend::External { path, sha256 } = &self.graphics {
            let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
            if !format!("{:x}", Sha256::digest(&bytes)).eq_ignore_ascii_case(sha256) {
                return Err("Graphics backend digest changed after selection".into());
            }
            crate::client_code::executable_sections(&bytes[..bytes.len().min(65536)], u32::MAX)?;
            let pe = u32::from_le_bytes(bytes[0x3c..0x40].try_into().map_err(|_| "Bad PE offset")?)
                as usize;
            let flags = u16::from_le_bytes(
                bytes[pe + 22..pe + 24]
                    .try_into()
                    .map_err(|_| "Bad PE flags")?,
            );
            if flags & 0x2000 == 0 {
                return Err("Graphics backend is not an i686 DLL".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn controller_discovery_is_default_on_for_old_options_and_can_be_disabled() {
        let mut options = super::RuntimeOptions::default();
        assert!(options.controller_discovery);
        let mut old = serde_json::to_value(&options).unwrap();
        old.as_object_mut().unwrap().remove("controller_discovery");
        assert!(
            serde_json::from_value::<super::RuntimeOptions>(old)
                .unwrap()
                .controller_discovery
        );
        options.controller_discovery = false;
        assert_eq!(
            super::RuntimeOptions::decode(&options.encode().unwrap()).unwrap(),
            options
        );
    }
    #[test]
    fn file_lab_pins_are_fixed_private_and_round_trip_without_enabling_general_addons() {
        use super::*;
        let folder = std::env::temp_dir();
        let keys = [
            "neighbor/helper.lua",
            "neighbor/neighbor.lua",
            "selected/helper.lua",
            "selected/load-error.lua",
            "selected/replacement.lua",
            "selected/selected.lua",
            "selected/syntax.lua",
        ];
        let files = LabFileProfile {
            path: folder.join("runtime-root/lab-profile.json"),
            sha256: "0".repeat(64),
            sources: keys
                .into_iter()
                .map(|k| (k.into(), "1".repeat(64)))
                .collect(),
        };
        let mut options = RuntimeOptions {
            client_diagnostic: Some(folder.join("diagnostic.json")),
            game_thread_probe: Some(GameThreadProbe {
                ready_report: folder.join("ready.json"),
                result_report: folder.join("result.json"),
                game_sha256: "0".repeat(64),
                addon_echo_trial: false,
                lab_test: Some(LabTest {
                    nonce: "a".repeat(32),
                    request: folder.join("request.json"),
                    scenario: LabScenario::ReloadWindower,
                    file_profile: Some(files),
                    profile_control: false,
                }),
            }),
            ..RuntimeOptions::default()
        };
        assert_eq!(
            RuntimeOptions::decode(&options.encode().unwrap()).unwrap(),
            options
        );
        let lab = options
            .game_thread_probe
            .as_mut()
            .unwrap()
            .lab_test
            .as_mut()
            .unwrap();
        lab.scenario = LabScenario::Persistent;
        assert!(options.validate().is_err());
        let lab = options
            .game_thread_probe
            .as_mut()
            .unwrap()
            .lab_test
            .as_mut()
            .unwrap();
        lab.scenario = LabScenario::ReloadWindower;
        lab.file_profile.as_mut().unwrap().path = folder.join("outside.json");
        assert!(options.validate().is_err());
        let lab = options
            .game_thread_probe
            .as_mut()
            .unwrap()
            .lab_test
            .as_mut()
            .unwrap();
        lab.file_profile.as_mut().unwrap().path = folder.join("runtime-root/lab-profile.json");
        lab.file_profile
            .as_mut()
            .unwrap()
            .sources
            .insert("selected/unexpected.lua".into(), "2".repeat(64));
        assert!(options.validate().is_err());
    }
    #[test]
    fn laboratory_scenarios_default_and_reject_unknown_operations() {
        use super::{LabScenario, LabTest};
        let value = serde_json::json!({"nonce":"a".repeat(32),"request":"C:/Test/request.json"});
        let lab: LabTest = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(lab.scenario, LabScenario::Persistent);
        assert_eq!(lab.scenario.operation(), "run_persistent");
        let mut unknown = value;
        unknown["scenario"] = serde_json::json!("run_arbitrary");
        assert!(serde_json::from_value::<LabTest>(unknown).is_err());
    }
    use super::*;
    #[test]
    fn game_probe_is_explicit_and_requires_distinct_reports_and_diagnostic_isolation() {
        let folder = std::env::temp_dir();
        let mut options = RuntimeOptions {
            game_thread_probe: Some(GameThreadProbe {
                ready_report: folder.join("ready.json"),
                result_report: folder.join("result.json"),
                game_sha256: "0".repeat(64),
                addon_echo_trial: false,
                lab_test: None,
            }),
            ..Default::default()
        };
        assert!(options.validate().is_err());
        options.client_diagnostic = Some(folder.join("diagnostic.json"));
        // Old prepared options remain a single-echo probe; opt-in survives IPC.
        let mut legacy = serde_json::to_value(&options).unwrap();
        legacy["game_thread_probe"]
            .as_object_mut()
            .unwrap()
            .remove("addon_echo_trial");
        assert!(
            !serde_json::from_value::<RuntimeOptions>(legacy)
                .unwrap()
                .game_thread_probe
                .unwrap()
                .addon_echo_trial
        );
        options.game_thread_probe.as_mut().unwrap().addon_echo_trial = true;
        assert_eq!(
            RuntimeOptions::decode(&options.encode().unwrap()).unwrap(),
            options
        );
        options.game_thread_probe.as_mut().unwrap().ready_report = folder.join("diagnostic.json");
        assert!(options.validate().is_err());
        options.game_thread_probe.as_mut().unwrap().ready_report = "relative.json".into();
        assert!(options.validate().is_err());
        options.game_thread_probe.as_mut().unwrap().ready_report = folder.join("ready.json");
        options.addons.push(AddonSpec {
            name: "unexpected".into(),
            ecosystem: Ecosystem::Native,
            engine: LuaEngine::Lua51,
        });
        assert!(options.validate().is_err());
    }
    #[test]
    fn first_client_diagnostic_rejects_addon_and_provider_loading() {
        let mut options = RuntimeOptions {
            client_diagnostic: Some(std::env::temp_dir().join("ffxi-diagnostic.json")),
            ..RuntimeOptions::default()
        };
        options.validate().unwrap();
        options.addons.push(AddonSpec {
            name: "unexpected".into(),
            ecosystem: Ecosystem::Native,
            engine: LuaEngine::Lua51,
        });
        assert!(options.validate().is_err());
        options.addons.clear();
        options.luajit_provider = Some(NativeLibrary {
            path: std::env::temp_dir().join("provider.dll"),
            sha256: "0".repeat(64),
        });
        assert!(options.validate().is_err());
        options.luajit_provider = None;
        options.client_diagnostic = Some("relative.json".into());
        assert!(options.validate().is_err());
    }
    #[test]
    fn lua_engine_selection_requires_a_pinned_provider_and_preserves_legacy_defaults() {
        let old: RuntimeOptions = serde_json::from_str(r#"{"schema_version":1,"graphics":{"kind":"system"},"addons":[{"name":"legacy","ecosystem":"windower4"}]}"#).unwrap();
        assert_eq!(old.addons[0].engine, LuaEngine::Lua51);
        let mut options = old;
        options.addons[0].engine = LuaEngine::LuaJit;
        assert!(options.encode().unwrap_err().contains("provider"));
        options.luajit_provider = Some(NativeLibrary {
            path: std::env::current_dir().unwrap().join("provider.dll"),
            sha256: "0".repeat(64),
        });
        let bytes = options.encode().unwrap();
        assert_eq!(RuntimeOptions::decode(&bytes).unwrap(), options);
        options.luajit_provider.as_mut().unwrap().sha256 = "invalid".into();
        assert!(options.validate().is_err());
    }
    #[test]
    fn options_round_trip_rejects_corruption_and_unknown_versions() {
        let options = RuntimeOptions::default();
        let bytes = options.encode().unwrap();
        assert_eq!(RuntimeOptions::decode(&bytes).unwrap(), options);
        for end in [0, 7, 8, 11, bytes.len() - 1] {
            assert!(RuntimeOptions::decode(&bytes[..end]).is_err());
        }
        let mut bad = bytes;
        bad[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(RuntimeOptions::decode(&bad).is_err());
        assert!(
            RuntimeOptions {
                schema_version: 2,
                ..options
            }
            .encode()
            .is_err()
        );
    }
    #[test]
    fn relative_backend_paths_and_missing_digests_are_rejected() {
        assert!(
            RuntimeOptions {
                graphics: GraphicsBackend::External {
                    path: "d3d8.dll".into(),
                    sha256: "0".repeat(64)
                },
                ..RuntimeOptions::default()
            }
            .validate()
            .is_err()
        );
        assert_ne!(mapping_name(10), mapping_name(11));
        assert_ne!(mapping_name(10), crate::protocol::SETTINGS_MAPPING);
    }
}
