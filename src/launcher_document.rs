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
        write_version(&directory, &self.name, |file| {
            file.write_all(&bytes).and_then(|_| file.sync_all())
        })
    }

    pub fn fingerprint(&self) -> Result<String, String> {
        Ok(digest(
            &serde_json::to_vec(self).map_err(|e| e.to_string())?,
        ))
    }
}

/// Complete and flush staging bytes before publishing a new, never-replaced name.
fn write_version(
    directory: &Path,
    name: &str,
    write: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
) -> Result<PathBuf, String> {
    struct Pending(PathBuf);
    impl Drop for Pending {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = directory.join(format!(
        ".signet-{}-{stamp}-{serial}.pending",
        std::process::id()
    ));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    // Only claim cleanup ownership after exclusive creation succeeds.
    let pending = Pending(path);
    {
        let mut file = file;
        write(&mut file)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
    }
    // Append after the newest existing revision, rather than filling archived gaps.
    let highest = std::fs::read_dir(directory)
        .map_err(|e| e.to_string())?
        .try_fold(0u32, |highest, entry| -> std::io::Result<u32> {
            let entry = entry?;
            let filename = entry.file_name();
            let number = filename
                .to_str()
                .and_then(|filename| filename.strip_suffix(".json"))
                .and_then(|stem| stem.rsplit_once('-'))
                .filter(|(profile, _)| profile.eq_ignore_ascii_case(name))
                .and_then(|(_, suffix)| suffix.parse::<u32>().ok())
                .filter(|number| (1..=10000).contains(number))
                .unwrap_or(0);
            Ok(highest.max(number))
        })
        .map_err(|e| e.to_string())?;
    for sequence in highest + 1..=10000 {
        let path = directory.join(format!("{name}-{sequence:04}.json"));
        match publish(&pending.0, &path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("Profile version capacity reached".into())
}
fn publish(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
        let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        // No REPLACE_EXISTING: another complete version always wins its name.
        if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    #[cfg(not(windows))]
    {
        std::fs::hard_link(from, to)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_partial_write_never_publishes_a_json_version() {
        let directory = std::env::temp_dir().join(format!(
            "signet-atomic-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let original = directory.join("Classic-0001.json");
        std::fs::write(&original, b"original").unwrap();
        let result = write_version(&directory, "Classic", |file| {
            file.write_all(b"{partial")?;
            Err(std::io::Error::other("injected interrupted write"))
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(&original).unwrap(), b"original");
        assert_eq!(
            std::fs::read_dir(&directory).unwrap().count(),
            1,
            "failed write left a partial final profile or a staging file"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn complete_version_is_published_without_replacing_existing_versions() {
        let directory = std::env::temp_dir().join(format!(
            "signet-publish-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let original = directory.join("Classic-0001.json");
        std::fs::write(&original, b"original").unwrap();
        let published = write_version(&directory, "Classic", |file| {
            assert_eq!(
                std::fs::read_dir(&directory)
                    .unwrap()
                    .filter_map(Result::ok)
                    .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
                    .count(),
                1,
                "final JSON was visible before its bytes were complete"
            );
            file.write_all(b"complete")
        })
        .unwrap();
        assert_eq!(published, directory.join("Classic-0002.json"));
        assert_eq!(std::fs::read(&original).unwrap(), b"original");
        assert_eq!(std::fs::read(&published).unwrap(), b"complete");
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 2);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn append_stays_newest_when_older_versions_have_been_archived() {
        let directory = std::env::temp_dir().join(format!(
            "signet-append-gap-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("Classic-0005.json"), b"current").unwrap();
        let published =
            write_version(&directory, "Classic", |file| file.write_all(b"newest")).unwrap();
        assert_eq!(
            published,
            directory.join("Classic-0006.json"),
            "an appended version must become the new head even when lower numbers are unused"
        );
        assert_eq!(
            std::fs::read(directory.join("Classic-0005.json")).unwrap(),
            b"current"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
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
