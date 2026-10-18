//! Offline, pinned packages and an append-only version store. No downloads.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_FILE: u64 = 16 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
    pub files: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub slot: String,
    pub manifest: Manifest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Generation {
    pub number: u64,
    pub previous: Option<u64>,
    pub slots: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mount {
    pub schema_version: u32,
    pub root: PathBuf,
    pub generation: Generation,
    pub files: BTreeMap<String, String>,
}

fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}
fn version(s: &str) -> bool {
    let parts: Vec<_> = s.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| {
            !p.is_empty()
                && (p.len() == 1 || !p.starts_with('0'))
                && p.bytes().all(|b| b.is_ascii_digit())
                && p.parse::<u32>().is_ok()
        })
}
fn relative(s: &str) -> bool {
    if s.is_empty() || s.len() > 512 || s.contains(['\\', ':', '\0']) || s.starts_with('/') {
        return false;
    }
    s.split('/').all(|p| {
        let base = p.split('.').next().unwrap_or("").to_ascii_uppercase();
        !p.is_empty()
            && p != "."
            && p != ".."
            && p.trim() == p
            && !p.ends_with('.')
            && p.len() <= 128
            && !p.chars().any(|c| c.is_control() || "<>\"|?*".contains(c))
            && !["CON", "PRN", "AUX", "NUL"].contains(&base.as_str())
            && !(base.len() == 4
                && (base.starts_with("COM") || base.starts_with("LPT"))
                && matches!(base.as_bytes()[3], b'1'..=b'9'))
    })
}
fn bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Expected a package file".into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("Package byte limit exceeded".into());
    }
    Ok(bytes)
}
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn contained(root: &Path, name: &str) -> Result<PathBuf, String> {
    if !relative(name) {
        return Err("Unsafe package relative path".into());
    }
    let canonical = root.canonicalize().map_err(|e| e.to_string())?;
    let mut cursor = canonical.clone();
    for part in name.split('/') {
        cursor.push(part);
        let meta = fs::symlink_metadata(&cursor).map_err(|e| e.to_string())?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err("Package reparse path refused".into());
            }
        }
        if meta.file_type().is_symlink() {
            return Err("Package symlink refused".into());
        }
    }
    let path = cursor.canonicalize().map_err(|e| e.to_string())?;
    if !path.starts_with(canonical) {
        return Err("Package path escaped its root".into());
    }
    Ok(path)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    f.write_all(bytes)
        .and_then(|_| f.sync_all())
        .map_err(|e| e.to_string())
}
fn json<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(value).map_err(|e| e.to_string())
}
impl Manifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
            || !identifier(&self.id)
            || !version(&self.version)
            || self.files.is_empty()
            || self.files.len() > 512
            || self.dependencies.len() > 64
            || self
                .dependencies
                .iter()
                .any(|(id, v)| !identifier(id) || !version(v))
        {
            return Err("Invalid pinned package manifest".into());
        }
        let mut insensitive = std::collections::BTreeSet::new();
        for (name, h) in &self.files {
            if !relative(name)
                || name.eq_ignore_ascii_case("receipt.json")
                || name.split('/').count() > 8
                || !insensitive.insert(name.to_lowercase())
                || h.len() != 64
                || !h
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            {
                return Err("Invalid/conflicting package file or digest".into());
            }
        }
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > 131072 {
            return Err("Package manifest byte limit".into());
        }
        Ok(())
    }
    pub fn read(path: &Path) -> Result<Self, String> {
        let value: Self =
            serde_json::from_slice(&bounded(path, 131072)?).map_err(|e| e.to_string())?;
        value.validate()?;
        Ok(value)
    }
}
/// Resolve exact pinned versions, with dependencies before dependents.
pub fn resolve(catalog: &[Manifest], roots: &[(String, String)]) -> Result<Vec<Manifest>, String> {
    if catalog.len() > 512 || roots.len() > 256 {
        return Err("Package catalog limit".into());
    }
    let mut indexed = BTreeMap::new();
    for m in catalog {
        m.validate()?;
        if indexed
            .insert((m.id.clone(), m.version.clone()), m)
            .is_some()
        {
            return Err("Ambiguous package version".into());
        }
    }
    fn visit(
        id: &str,
        v: &str,
        catalog: &BTreeMap<(String, String), &Manifest>,
        chosen: &mut BTreeMap<String, String>,
        active: &mut Vec<String>,
        out: &mut Vec<Manifest>,
    ) -> Result<(), String> {
        if active.iter().any(|s| s == id) {
            return Err("Package dependency cycle".into());
        }
        if let Some(existing) = chosen.get(id) {
            return if existing == v {
                Ok(())
            } else {
                Err("Conflicting package versions".into())
            };
        }
        let m = catalog
            .get(&(id.into(), v.into()))
            .ok_or("Missing pinned package dependency")?;
        active.push(id.into());
        for (dep, version) in &m.dependencies {
            visit(dep, version, catalog, chosen, active, out)?;
        }
        active.pop();
        chosen.insert(id.into(), v.into());
        out.push((*m).clone());
        Ok(())
    }
    let mut chosen = BTreeMap::new();
    let mut out = Vec::new();
    for (id, v) in roots {
        visit(id, v, &indexed, &mut chosen, &mut Vec::new(), &mut out)?;
    }
    Ok(out)
}

pub struct Store {
    root: PathBuf,
}
impl Store {
    pub fn open(root: &Path) -> Result<Self, String> {
        if !root.is_absolute() {
            return Err("Package store needs an explicit absolute directory".into());
        }
        fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        for name in ["slots", "generations"] {
            fs::create_dir_all(root.join(name)).map_err(|e| e.to_string())?;
            contained(&root, name)?;
        }
        Ok(Self { root })
    }
    pub fn stage(&self, manifest: &Manifest, source: &Path) -> Result<Receipt, String> {
        self.stage_with_progress(manifest, source, |_| Ok(()))
    }
    pub fn stage_with_progress(
        &self,
        manifest: &Manifest,
        source: &Path,
        mut progress: impl FnMut(usize) -> Result<(), String>,
    ) -> Result<Receipt, String> {
        manifest.validate()?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let slot = format!(
            "slots/pkg-{}/{}/{}-{}",
            manifest.id,
            manifest.version,
            std::process::id(),
            stamp
        );
        let path = self.root.join(&slot);
        safe_directories(
            &self.root,
            Path::new(&slot).parent().ok_or("Missing slot parent")?,
        )?;
        fs::create_dir(&path).map_err(|e| e.to_string())?;
        let mut total = 0usize;
        for (index, (name, hash)) in manifest.files.iter().enumerate() {
            let bytes = bounded(&contained(source, name)?, MAX_FILE)?;
            total += bytes.len();
            if total > 128 * 1024 * 1024 || digest(&bytes) != *hash {
                return Err("Package source digest/total limit mismatch".into());
            }
            let target = path.join(name);
            safe_directories(
                &path,
                Path::new(name).parent().ok_or("Missing file parent")?,
            )?;
            write_new(&target, &bytes)?;
            progress(index + 1)?;
        }
        let receipt = Receipt {
            slot,
            manifest: manifest.clone(),
        };
        write_new(&path.join("receipt.json"), &json(&receipt)?)?;
        self.verify_slot(&receipt.slot)?;
        Ok(receipt)
    }
    pub fn verify_slot(&self, slot: &str) -> Result<Receipt, String> {
        if !slot.starts_with("slots/") || !relative(slot) {
            return Err("Invalid package slot".into());
        }
        let path = contained(&self.root, &format!("{slot}/receipt.json"))?;
        let receipt: Receipt =
            serde_json::from_slice(&bounded(&path, 131072)?).map_err(|e| e.to_string())?;
        receipt.manifest.validate()?;
        if receipt.slot != slot {
            return Err("Package slot identity mismatch".into());
        }
        let base = path.parent().ok_or("Missing package directory")?;
        let mut total = 0usize;
        for (name, h) in &receipt.manifest.files {
            let bytes = bounded(&contained(base, name)?, MAX_FILE)?;
            total += bytes.len();
            if total > 128 * 1024 * 1024 || digest(&bytes) != *h {
                return Err("Installed package file digest mismatch".into());
            }
        }
        Ok(receipt)
    }
    pub fn current(&self) -> Result<Option<Generation>, String> {
        let pointer = self.root.join("CURRENT");
        if !pointer.exists() {
            return Ok(None);
        }
        let number = String::from_utf8(bounded(&contained(&self.root, "CURRENT")?, 32)?)
            .map_err(|e| e.to_string())?
            .trim()
            .parse::<u64>()
            .map_err(|_| "Invalid package generation pointer")?;
        self.generation(number).map(Some)
    }
    pub fn generation(&self, number: u64) -> Result<Generation, String> {
        let g: Generation = serde_json::from_slice(&bounded(
            &contained(&self.root, &format!("generations/{number:08}.json"))?,
            131072,
        )?)
        .map_err(|e| e.to_string())?;
        if g.number != number || g.slots.len() > 256 {
            return Err("Invalid package generation".into());
        }
        let mut manifests = Vec::new();
        for (id, slot) in &g.slots {
            let receipt = self.verify_slot(slot)?;
            if receipt.manifest.id != *id {
                return Err("Package generation id mismatch".into());
            }
            manifests.push(receipt.manifest);
        }
        resolve(
            &manifests,
            &manifests
                .iter()
                .map(|m| (m.id.clone(), m.version.clone()))
                .collect::<Vec<_>>(),
        )?;
        Ok(g)
    }
    pub fn activate(&self, slots: &[String]) -> Result<Generation, String> {
        let _lock = StoreLock::acquire(&self.root)?;
        let previous = self.current()?.map(|g| g.number);
        self.activate_locked(slots, previous)
    }
    pub fn rollback(&self, number: u64) -> Result<Generation, String> {
        let _lock = StoreLock::acquire(&self.root)?;
        let desired = self.generation(number)?;
        self.activate_locked(
            &desired.slots.values().cloned().collect::<Vec<_>>(),
            self.current()?.map(|g| g.number),
        )
    }
    /// Create a new, verified runtime root from one locked generation snapshot.
    /// Nothing is published or selected if copying fails; existing roots are refused.
    pub fn prepare_runtime(&self, destination: &Path) -> Result<Mount, String> {
        let _lock = StoreLock::acquire(&self.root)?;
        let generation = self
            .current()?
            .ok_or("Activate packages before preparing addon files")?;
        if generation.slots.is_empty() || !destination.is_absolute() || destination.exists() {
            return Err("Choose a new absolute runtime directory for a nonempty selection".into());
        }
        let mut files = BTreeMap::new();
        let mut insensitive = std::collections::BTreeSet::new();
        let mut snapshots = Vec::new();
        let mut total = 0usize;
        for slot in generation.slots.values() {
            let receipt = self.verify_slot(slot)?;
            let base = contained(&self.root, slot)?;
            for (name, hash) in &receipt.manifest.files {
                let lower = name.to_lowercase();
                if lower == ".rust-mount.json"
                    || lower.starts_with(".rust-mount.json/")
                    || !insensitive.insert(lower)
                    || files.len() >= 4096
                {
                    return Err("Conflicting runtime package files or file count limit".into());
                }
                let bytes = bounded(&contained(&base, name)?, MAX_FILE)?;
                total += bytes.len();
                if total > 256 * 1024 * 1024 || digest(&bytes) != *hash {
                    return Err("Runtime package snapshot digest/total limit mismatch".into());
                }
                files.insert(name.clone(), hash.clone());
                snapshots.push((name.clone(), bytes));
            }
        }
        // Reject file/directory collisions before creating even a partial root.
        for name in &insensitive {
            let mut parent = name.as_str();
            while let Some((prefix, _)) = parent.rsplit_once('/') {
                if insensitive.contains(prefix) {
                    return Err("Conflicting runtime package file/directory paths".into());
                }
                parent = prefix;
            }
        }
        fs::create_dir(destination).map_err(|e| e.to_string())?;
        for (name, bytes) in snapshots {
            safe_directories(
                destination,
                Path::new(&name)
                    .parent()
                    .ok_or("Missing runtime file parent")?,
            )?;
            write_new(&destination.join(name), &bytes)?;
        }
        let mount = Mount {
            schema_version: 1,
            root: destination.to_path_buf(),
            generation,
            files,
        };
        // Validate the completed copied bytes before issuing the completion receipt.
        for (name, hash) in &mount.files {
            if digest(&bounded(&contained(destination, name)?, MAX_FILE)?) != *hash {
                return Err("Copied runtime package digest mismatch".into());
            }
        }
        write_new(&destination.join(".rust-mount.json"), &json(&mount)?)?;
        Ok(mount)
    }
    fn activate_locked(
        &self,
        slots: &[String],
        previous: Option<u64>,
    ) -> Result<Generation, String> {
        if slots.len() > 256 {
            return Err("Too many active packages".into());
        }
        let mut selected = BTreeMap::new();
        let mut manifests = Vec::new();
        for slot in slots {
            let receipt = self.verify_slot(slot)?;
            if selected
                .insert(receipt.manifest.id.clone(), slot.clone())
                .is_some()
            {
                return Err("Conflicting active package ids".into());
            }
            manifests.push(receipt.manifest);
        }
        resolve(
            &manifests,
            &manifests
                .iter()
                .map(|m| (m.id.clone(), m.version.clone()))
                .collect::<Vec<_>>(),
        )?;
        let max = fs::read_dir(self.root.join("generations"))
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok())
            .filter_map(|e| e.path().file_stem()?.to_str()?.parse::<u64>().ok())
            .max()
            .unwrap_or(0);
        let number = max.checked_add(1).ok_or("Package generation overflow")?;
        let generation = Generation {
            number,
            previous,
            slots: selected,
        };
        write_new(
            &self.root.join(format!("generations/{number:08}.json")),
            &json(&generation)?,
        )?;
        let pending = self.root.join(format!("CURRENT.{number:08}.pending"));
        write_new(&pending, format!("{number}\n").as_bytes())?;
        replace_pointer(&pending, &self.root.join("CURRENT"))?;
        Ok(generation)
    }
}
fn safe_directories(root: &Path, relative_path: &Path) -> Result<(), String> {
    let mut name = String::new();
    for component in relative_path.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err("Invalid package directory component".into());
        }
        if !name.is_empty() {
            name.push('/');
        }
        name.push_str(
            component
                .as_os_str()
                .to_str()
                .ok_or("Non-UTF8 package directory")?,
        );
        let path = root.join(&name);
        match fs::create_dir(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.to_string()),
        }
        if !contained(root, &name)?.is_dir() {
            return Err("Package parent is not a directory".into());
        }
    }
    Ok(())
}
#[cfg(windows)]
struct StoreLock(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl StoreLock {
    fn acquire(root: &Path) -> Result<Self, String> {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, WAIT_ABANDONED, WAIT_OBJECT_0},
            System::Threading::{CreateMutexW, WaitForSingleObject},
        };
        let name: Vec<_> = format!(
            "Local\\WindowerRust.PackageStore.{}",
            digest(root.to_string_lossy().to_lowercase().as_bytes())
        )
        .encode_utf16()
        .chain(Some(0))
        .collect();
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err("Cannot acquire package store lock".into());
        }
        let result = unsafe { WaitForSingleObject(handle, 5000) };
        if result != WAIT_OBJECT_0 && result != WAIT_ABANDONED {
            unsafe {
                CloseHandle(handle);
            }
            return Err("Package store is busy".into());
        }
        Ok(Self(handle))
    }
}
#[cfg(windows)]
impl Drop for StoreLock {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(self.0);
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
#[cfg(not(windows))]
struct StoreLock(std::sync::MutexGuard<'static, ()>);
#[cfg(not(windows))]
impl StoreLock {
    fn acquire(_: &Path) -> Result<Self, String> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock()
            .map(Self)
            .map_err(|_| "Package store lock poisoned".into())
    }
}
#[cfg(windows)]
fn replace_pointer(from: &Path, to: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    let wide = |p: &Path| {
        p.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    if unsafe {
        windows_sys::Win32::Storage::FileSystem::MoveFileExW(
            wide(from).as_ptr(),
            wide(to).as_ptr(),
            0x1 | 0x8,
        )
    } == 0
    {
        return Err("Atomic package activation failed".into());
    }
    Ok(())
}
#[cfg(not(windows))]
fn replace_pointer(from: &Path, to: &Path) -> Result<(), String> {
    fs::rename(from, to).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest(id: &str, v: &str) -> Manifest {
        Manifest {
            schema_version: 1,
            id: id.into(),
            version: v.into(),
            dependencies: BTreeMap::new(),
            files: BTreeMap::from([("a.lua".into(), digest(b"return 1"))]),
        }
    }
    fn directory(label: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "rust-packages-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&p).unwrap();
        p
    }
    #[test]
    fn prepared_runtime_pins_generation_preserves_old_roots_and_rejects_collisions() {
        let root = directory("mount");
        let src = directory("mount-source");
        std::fs::write(src.join("a.lua"), b"return 1").unwrap();
        let store = Store::open(&root).unwrap();
        let first = store.stage(&manifest("addon", "1.0.0"), &src).unwrap();
        store.activate(std::slice::from_ref(&first.slot)).unwrap();
        let destination = root.join("runtime-one");
        let mounted = store.prepare_runtime(&destination).unwrap();
        assert_eq!(mounted.generation.number, 1);
        assert_eq!(
            std::fs::read(destination.join("a.lua")).unwrap(),
            b"return 1"
        );
        assert!(destination.join(".rust-mount.json").is_file());
        assert!(store.prepare_runtime(&destination).is_err());
        // Cross-package Windows path collisions must fail before creating a root.
        let mut other = manifest("other", "1.0.0");
        other.files = std::collections::BTreeMap::from([("A.lua".into(), digest(b"return 1"))]);
        std::fs::write(src.join("A.lua"), b"return 1").unwrap();
        let second = store.stage(&other, &src).unwrap();
        store.activate(&[first.slot.clone(), second.slot]).unwrap();
        let collision = root.join("must-not-exist");
        assert!(store.prepare_runtime(&collision).is_err());
        assert!(!collision.exists());
        assert_eq!(
            std::fs::read(destination.join("a.lua")).unwrap(),
            b"return 1"
        );
        store.activate(std::slice::from_ref(&first.slot)).unwrap();
        std::fs::write(root.join(first.slot).join("a.lua"), b"corrupt").unwrap();
        let corrupt = root.join("must-not-exist-corrupt");
        assert!(store.prepare_runtime(&corrupt).is_err());
        assert!(!corrupt.exists());
        assert_eq!(mounted.files["a.lua"], digest(b"return 1"));
    }
    #[test]
    fn resolver_orders_exact_dependencies_and_refuses_cycles_conflicts_missing() {
        let lib = manifest("lib", "1.0.0");
        let mut app = manifest("app", "1.0.0");
        app.dependencies.insert("lib".into(), "1.0.0".into());
        assert_eq!(
            resolve(
                &[app.clone(), lib.clone()],
                &[("app".into(), "1.0.0".into())]
            )
            .unwrap()
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
            vec!["lib", "app"]
        );
        assert!(resolve(&[app.clone()], &[("app".into(), "1.0.0".into())]).is_err());
        let lib2 = manifest("lib", "2.0.0");
        assert!(
            resolve(
                &[app.clone(), lib.clone(), lib2],
                &[
                    ("app".into(), "1.0.0".into()),
                    ("lib".into(), "2.0.0".into())
                ]
            )
            .is_err()
        );
        let mut cycle = lib;
        cycle.dependencies.insert("app".into(), "1.0.0".into());
        assert!(resolve(&[app, cycle], &[("app".into(), "1.0.0".into())]).is_err());
    }
    #[test]
    fn package_interruption_digest_failure_and_rollback_preserve_old_versions() {
        let root = directory("store");
        let source = directory("source");
        fs::write(source.join("a.lua"), b"return 1").unwrap();
        let store = Store::open(&root).unwrap();
        let first = store.stage(&manifest("test", "1.0.0"), &source).unwrap();
        let old = store.activate(std::slice::from_ref(&first.slot)).unwrap();
        assert!(
            store
                .stage_with_progress(&manifest("test", "2.0.0"), &source, |_| Err(
                    "cancelled".into()
                ))
                .is_err()
        );
        assert_eq!(store.current().unwrap().unwrap().number, old.number);
        let mut bad = manifest("test", "2.0.0");
        bad.files.insert("a.lua".into(), "0".repeat(64));
        assert!(store.stage(&bad, &source).is_err());
        let second = store.stage(&manifest("test", "2.0.0"), &source).unwrap();
        let new = store.activate(std::slice::from_ref(&second.slot)).unwrap();
        assert!(new.number > old.number);
        assert!(store.verify_slot(&first.slot).is_ok());
        let rollback = store.rollback(old.number).unwrap();
        assert_eq!(rollback.slots, old.slots);
        assert_eq!(rollback.previous, Some(new.number));
        fs::write(root.join(&second.slot).join("a.lua"), b"tampered").unwrap();
        assert!(store.activate(&[second.slot]).is_err());
        assert_eq!(store.current().unwrap().unwrap().number, rollback.number);
    }
    #[test]
    fn manifests_refuse_traversal_reserved_names_and_windows_collisions() {
        for path in [
            "../x", "/x", "a\\x", "a:stream", "NUL.txt", "dir/COM1", "a./x", "a//x",
        ] {
            let mut m = manifest("test", "1.0.0");
            m.files = BTreeMap::from([(path.into(), "0".repeat(64))]);
            assert!(m.validate().is_err(), "{path}");
        }
        let mut m = manifest("test", "1.0.0");
        m.files.insert("A.lua".into(), "0".repeat(64));
        assert!(m.validate().is_err());
    }
}
