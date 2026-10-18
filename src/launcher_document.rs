//! Versioned local launcher workspace. Saving never replaces an existing profile.
use crate::{addon_profile::AddonProfile, packages::digest};
use serde::{Deserialize, Serialize};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub schema_version: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub ui_scale: f32,
    pub profile: AddonProfile,
}
impl Document {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
            || self.name.is_empty()
            || self.name.len() > 64
            || !self
                .name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            return Err("Profile name: use 1-64 letters, numbers, hyphens or underscores".into());
        }
        if !(640..=16384).contains(&self.width)
            || !(480..=16384).contains(&self.height)
            || !self.ui_scale.is_finite()
            || !(0.5..=4.0).contains(&self.ui_scale)
        {
            return Err("Display: width 640-16384, height 480-16384, UI scale 0.5-4".into());
        }
        self.profile.validate()
    }
    pub fn validate_files(&self) -> Result<Vec<(String, String)>, String> {
        self.validate()?;
        if let Some(provider) = &self.profile.luajit_provider {
            provider.verify()?;
        }
        self.profile
            .addons
            .iter()
            .map(|a| {
                let source = self.profile.resolve(&a.name)?;
                Ok((a.name.clone(), source.sha256))
            })
            .collect()
    }
    pub fn read(path: &Path) -> Result<Self, String> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(|e| e.to_string())?
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 65536 {
            return Err("Launcher profile exceeds 64 KiB".into());
        }
        let doc: Self = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        doc.validate()?;
        Ok(doc)
    }
    pub fn save_new(&self, workspace: &Path) -> Result<PathBuf, String> {
        self.validate_files()?;
        if !workspace.is_absolute() || !workspace.is_dir() {
            return Err("Select an existing absolute launcher workspace".into());
        }
        let root = workspace.canonicalize().map_err(|e| e.to_string())?;
        let directory = root.join("profiles");
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        if directory.canonicalize().map_err(|e| e.to_string())? != directory {
            return Err("Launcher profile directory redirects outside its workspace".into());
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        if bytes.len() > 65536 {
            return Err("Launcher profile exceeds 64 KiB".into());
        }
        for sequence in 1..=10000 {
            let path = directory.join(format!("{}-{sequence:04}.json", self.name));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    file.write_all(&bytes)
                        .and_then(|_| file.sync_all())
                        .map_err(|e| e.to_string())?;
                    return Ok(path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
        Err("Profile version capacity reached".into())
    }
    pub fn fingerprint(&self) -> Result<String, String> {
        Ok(digest(
            &serde_json::to_vec(self).map_err(|e| e.to_string())?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_display_and_profile_paths_are_rejected_before_save() {
        let mut d = Document {
            schema_version: 1,
            name: "Lab".into(),
            width: 3840,
            height: 2160,
            ui_scale: 1.5,
            profile: AddonProfile {
                schema_version: 1,
                root: std::env::temp_dir(),
                addons: vec![],
                luajit_provider: None,
            },
        };
        assert!(d.validate().is_ok());
        for bad in ["../escape", "CON?", "", "x/y"] {
            d.name = bad.into();
            assert!(d.validate().is_err());
        }
        d.name = "Lab".into();
        d.ui_scale = f32::NAN;
        assert!(d.validate().is_err());
        d.ui_scale = 1.5;
        d.width = 0;
        assert!(d.validate().is_err());
        d.width = 3840;
        d.profile.root = PathBuf::from("relative");
        assert!(d.validate().is_err());
    }
}
