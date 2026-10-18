//! Explicit trusted Lua file selections. Path checks are not a Lua sandbox.
use crate::{
    platform::Ecosystem,
    runtime_options::{AddonSpec, LuaEngine, RuntimeOptions},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

pub const SOURCE_LIMIT: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddonProfile {
    pub schema_version: u32,
    pub root: PathBuf,
    pub addons: Vec<AddonSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub luajit_provider: Option<crate::runtime_options::NativeLibrary>,
}

pub struct Source {
    pub name: String,
    pub ecosystem: Ecosystem,
    pub engine: LuaEngine,
    pub runtime_path: String,
    pub addon_path: String,
    pub text: String,
    pub sha256: String,
}

fn lua_directory(path: &Path) -> Result<String, String> {
    let text = path.to_string_lossy();
    let normalized = if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        format!("//{}", unc.replace('\\', "/"))
    } else {
        text.strip_prefix(r"\\?\")
            .unwrap_or(&text)
            .replace('\\', "/")
    };
    if normalized.contains(['?', ';']) {
        return Err("Addon directory cannot contain Lua search-path delimiters".into());
    }
    Ok(format!("{}/", normalized.trim_end_matches('/')))
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Expected a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("Addon profile/source exceeds its byte limit".into());
    }
    Ok(bytes)
}

impl AddonProfile {
    pub fn read(path: &Path) -> Result<Self, String> {
        Self::read_checked(path, None)
    }
    pub fn read_verified(path: &Path, expected: &str) -> Result<Self, String> {
        Self::read_checked(path, Some(expected))
    }
    fn read_checked(path: &Path, expected: Option<&str>) -> Result<Self, String> {
        let bytes = read_bounded(path, 65536)?;
        if let Some(expected) = expected
            && format!("{:x}", Sha256::digest(&bytes)) != expected
        {
            return Err("Prepared addon profile digest changed".into());
        }
        let profile: Self = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        profile.validate()?;
        Ok(profile)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 || !self.root.is_absolute() {
            return Err("Addon profile requires schema 1 and an absolute root".into());
        }
        RuntimeOptions {
            addons: self.addons.clone(),
            luajit_provider: self.luajit_provider.clone(),
            ..RuntimeOptions::default()
        }
        .validate()?;
        for addon in &self.addons {
            let name = &addon.name;
            let base = name.split('.').next().unwrap_or("").to_ascii_uppercase();
            let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&base.as_str())
                || (base.len() == 4
                    && (base.starts_with("COM") || base.starts_with("LPT"))
                    && matches!(base.as_bytes()[3], b'1'..=b'9'));
            if name.len() > 256
                || name.trim() != name
                || name.ends_with('.')
                || name
                    .chars()
                    .any(|c| c.is_control() || "<>\"|?*".contains(c))
                || reserved
            {
                return Err("Unsupported Lua engine or invalid addon directory name".into());
            }
        }
        Ok(())
    }
    pub fn resolve(&self, name: &str) -> Result<Source, String> {
        let spec = self
            .addons
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
            .ok_or("Addon is not selected in this profile")?;
        self.resolve_entry(name, &format!("{}.lua", spec.name))
    }
    /// Read a candidate leaf inside a selected addon directory, never an arbitrary path.
    pub fn resolve_entry(&self, name: &str, entry: &str) -> Result<Source, String> {
        self.validate()?;
        if entry.is_empty()
            || entry.len() > 256
            || !entry.ends_with(".lua")
            || !entry
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err("Invalid addon candidate leaf".into());
        }
        let spec = self
            .addons
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
            .ok_or("Addon is not selected in this profile")?;
        let root = self.root.canonicalize().map_err(|e| e.to_string())?;
        let addons = root
            .join("addons")
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let directory = addons
            .join(&spec.name)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let file = directory
            .join(entry)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if !addons.starts_with(&root)
            || !directory.starts_with(&addons)
            || !file.starts_with(&directory)
        {
            return Err("Addon source escapes the selected root/directory".into());
        }
        let bytes = read_bounded(&file, SOURCE_LIMIT)?;
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        let text = String::from_utf8(bytes).map_err(|_| "Addon source must be UTF-8")?;
        Ok(Source {
            name: spec.name.clone(),
            ecosystem: spec.ecosystem,
            engine: spec.engine,
            runtime_path: lua_directory(&root)?,
            addon_path: lua_directory(&directory)?,
            text,
            sha256,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn candidate_leaf_refuses_paths_before_any_file_read() {
        for leaf in [
            "../escape.lua",
            "folder/file.lua",
            r"folder\file.lua",
            "C:escape.lua",
            "bad;entry.lua",
            "not-lua.txt",
        ] {
            assert!(
                profile("selected")
                    .resolve_entry("selected", leaf)
                    .err()
                    .unwrap()
                    .contains("candidate leaf")
            );
        }
    }
    fn profile(name: &str) -> AddonProfile {
        AddonProfile {
            schema_version: 1,
            luajit_provider: None,
            root: std::env::current_dir().unwrap(),
            addons: vec![AddonSpec {
                name: name.into(),
                ecosystem: Ecosystem::Native,
                engine: LuaEngine::Lua51,
            }],
        }
    }
    #[test]
    fn profile_refuses_ambiguous_windows_names_and_unsupported_engines() {
        for name in [
            "..",
            "CON",
            "con.txt",
            "LPT1",
            "COM9",
            "trailing.",
            " spaced",
            "a?b",
            "a:b",
            "a/b",
            "a\0b",
        ] {
            assert!(profile(name).validate().is_err(), "accepted {name:?}");
        }
        let mut p = profile("Selected");
        p.addons.push(AddonSpec {
            name: "selected".into(),
            ..p.addons[0].clone()
        });
        assert!(p.validate().is_err());
        let mut p = profile("Selected");
        p.addons[0].engine = LuaEngine::LuaJit;
        assert!(p.validate().is_err());
        p.addons[0].engine = LuaEngine::Lua51;
        p.addons[0].ecosystem = Ecosystem::Windower5;
        assert!(p.validate().is_err());
    }
    #[test]
    fn profile_and_selection_schema_reject_unknown_fields() {
        let mut value = serde_json::to_value(profile("Selected")).unwrap();
        value["execute"] = serde_json::json!("unexpected");
        assert!(serde_json::from_value::<AddonProfile>(value).is_err());
        let mut value = serde_json::to_value(profile("Selected")).unwrap();
        value["addons"][0]["path"] = serde_json::json!("../unexpected.lua");
        assert!(serde_json::from_value::<AddonProfile>(value).is_err());
    }
}
#[test]
fn lua_search_paths_strip_windows_verbatim_prefix_and_reject_delimiters() {
    assert_eq!(
        lua_directory(Path::new(r"\\?\F:\Mods\addons\Selected")).unwrap(),
        "F:/Mods/addons/Selected/"
    );
    assert_eq!(
        lua_directory(Path::new(r"\\?\UNC\server\share\Mods")).unwrap(),
        "//server/share/Mods/"
    );
    assert!(lua_directory(Path::new("F:/bad;directory")).is_err());
    assert!(lua_directory(Path::new("F:/bad?directory")).is_err());
}
