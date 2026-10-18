//! Player contract v1: immutable native documents, optimistic revisions and one writer.
use crate::{launcher_document::Document, runtime_options::AddonSpec};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
#[derive(Deserialize)]
#[serde(try_from = "Value")]
pub struct Request {
    pub schema_version: u32,
    pub operation: Operation,
}
impl TryFrom<Value> for Request {
    type Error = String;
    fn try_from(mut value: Value) -> Result<Self, String> {
        let version = value
            .as_object_mut()
            .ok_or("Expected a player request")?
            .remove("schema_version")
            .and_then(|v| v.as_u64())
            .ok_or("Player contract version is required")?;
        let schema_version = u32::try_from(version).map_err(|_| "Invalid contract version")?;
        Ok(Self {
            schema_version,
            operation: serde_json::from_value(value).map_err(|e| e.to_string())?,
        })
    }
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    SaveProfile {
        id: String,
        name: String,
        width: u32,
        height: u32,
        ui_scale: f32,
        enabled: Vec<String>,
    },
    CopyProfile {
        id: String,
        name: String,
    },
    RestoreProfile {
        id: String,
        version: String,
    },
}
#[derive(Serialize)]
struct ProfileView {
    id: String,
    document: Document,
    available_addons: Vec<AddonSpec>,
    history: Vec<String>,
    requires_restore: bool,
}
const PROFILE_CAPACITY: usize = 4096;
fn revision(id: &str) -> Option<(&str, u32)> {
    let (name, suffix) = id.rsplit_once('-')?;
    let number = suffix.parse::<u32>().ok()?;
    ((1..=10000).contains(&number) && suffix == format!("{number:04}")).then_some((name, number))
}
struct Documents {
    valid: BTreeMap<String, Document>,
    ids: BTreeSet<String>,
    warnings: Vec<String>,
}
pub struct Player {
    root: PathBuf,
    lock: PathBuf,
}
impl Drop for Player {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.lock);
    }
}
fn plain(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err("Redirected player workspace path refused".into());
        }
    }
    if meta.file_type().is_symlink() {
        return Err("Redirected player workspace path refused".into());
    }
    Ok(())
}
impl Player {
    pub fn open(root: &Path) -> Result<Self, String> {
        if !root.is_absolute() || !root.is_dir() {
            return Err("Select an existing absolute player workspace".into());
        }
        plain(root)?;
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        let profiles = root.join("profiles");
        fs::create_dir_all(&profiles).map_err(|e| e.to_string())?;
        plain(&profiles)?;
        let lock = root.join(".signet-player.lock");
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
            .map_err(
                |_| "Player workspace is already open, or its previous lock needs inspection",
            )?;
        writeln!(f, "{}", std::process::id()).map_err(|e| e.to_string())?;
        Ok(Self { root, lock })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    fn scan(&self) -> Result<Documents, String> {
        let mut result = Documents {
            valid: BTreeMap::new(),
            ids: BTreeSet::new(),
            warnings: Vec::new(),
        };
        plain(&self.root.join("profiles"))?;
        for entry in fs::read_dir(self.root.join("profiles")).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or("Invalid profile filename")?
                .to_string();
            if result.ids.len() >= PROFILE_CAPACITY {
                return Err("Player workspace profile capacity reached".into());
            }
            result.ids.insert(id.clone());
            match plain(&path)
                .and_then(|_| Document::read(&path))
                .and_then(|doc| {
                    if revision(&id).is_none_or(|(name, _)| name != doc.name) {
                        return Err("Filename does not match its profile version".into());
                    }
                    Ok(doc)
                }) {
                Ok(doc) => {
                    result.valid.insert(id, doc);
                }
                Err(_) => result
                    .warnings
                    .push(format!("{id}: unreadable profile version preserved")),
            }
        }
        Ok(result)
    }
    fn views(&self) -> Result<Vec<ProfileView>, String> {
        let documents = self.scan()?;
        Ok(Self::views_from(documents.valid, &documents.ids))
    }
    fn views_from(docs: BTreeMap<String, Document>, ids: &BTreeSet<String>) -> Vec<ProfileView> {
        let mut groups: BTreeMap<String, Vec<(String, Document)>> = BTreeMap::new();
        let catalogs: Vec<_> = docs.values().map(|d| d.profile.clone()).collect();
        for (id, doc) in docs {
            groups
                .entry(doc.name.to_ascii_lowercase())
                .or_default()
                .push((id, doc));
        }
        let mut out = Vec::new();
        for versions in groups.values_mut() {
            versions.sort_by_key(|(id, _)| revision(id).map(|(_, n)| n));
            let (id, doc) = versions.last().unwrap();
            let mut catalog = BTreeMap::new();
            // The library is shared by native runtime root; selections remain per profile.
            for version in &catalogs {
                if version.root != doc.profile.root {
                    continue;
                }
                for addon in &version.addons {
                    catalog.insert(addon.name.to_lowercase(), addon.clone());
                }
            }
            // A profile's explicit engine/ecosystem always overrides the shared library.
            for (_, version) in versions.iter() {
                if version.profile.root == doc.profile.root {
                    for addon in &version.profile.addons {
                        catalog.insert(addon.name.to_lowercase(), addon.clone());
                    }
                }
            }
            out.push(ProfileView {
                id: id.clone(),
                requires_restore: ids.iter().any(|candidate| {
                    revision(candidate).is_some_and(|(name, number)| {
                        name.eq_ignore_ascii_case(&doc.name) && number > revision(id).unwrap().1
                    })
                }),
                document: doc.clone(),
                available_addons: catalog.into_values().collect(),
                history: versions.iter().map(|(id, _)| id.clone()).collect(),
            });
        }
        out
    }
    pub fn snapshot(&self) -> Result<Value, String> {
        let documents = self.scan()?;
        Ok(
            json!({"schema_version":1,"profiles":Self::views_from(documents.valid, &documents.ids),"warnings":documents.warnings,"game_launch":{"available":false,"reason":"Local game launch requires verified lifetime private USER isolation for this Rust client."},"capabilities":{"native_profiles":true,"package_store":true,"asset_overrides":false,"graphics_setup":false,"native_plugins":false,"windower":"partial","ashita":"partial"}}),
        )
    }
    pub fn profile(&self, id: &str) -> Result<Document, String> {
        self.scan()?
            .valid
            .remove(id)
            .ok_or_else(|| "Profile is outside this workspace or no longer exists".into())
    }
    /// Starting work requires the newest revision, including any damaged final file.
    pub fn current_profile(&self, id: &str) -> Result<Document, String> {
        let documents = self.scan()?;
        let doc = documents
            .valid
            .get(id)
            .ok_or("Profile is outside this workspace or no longer exists")?;
        let (_, selected_revision) = revision(id).ok_or("Invalid profile revision")?;
        if documents.ids.iter().any(|candidate| {
            revision(candidate).is_some_and(|(name, number)| {
                name.eq_ignore_ascii_case(&doc.name) && number > selected_revision
            })
        }) {
            return Err("Profile changed or has a damaged newer version; refresh or restore before starting".into());
        }
        let doc = doc.clone();
        Ok(doc)
    }
    pub fn execute(&self, request: Request) -> Result<Value, String> {
        if request.schema_version != 1 {
            return Err("Unsupported player contract version".into());
        }
        let id = match &request.operation {
            Operation::SaveProfile { id, .. }
            | Operation::CopyProfile { id, .. }
            | Operation::RestoreProfile { id, .. } => id,
        };
        let views = self.views()?;
        let current = views
            .iter()
            .find(|v| v.id == *id)
            .ok_or("Profile changed; refresh before saving")?;
        if !matches!(&request.operation, Operation::RestoreProfile { .. }) {
            self.current_profile(id)?;
        }
        let mut doc = current.document.clone();
        match request.operation {
            Operation::SaveProfile {
                name,
                width,
                height,
                ui_scale,
                enabled,
                ..
            } => {
                if name != doc.name
                    && views
                        .iter()
                        .any(|v| v.document.name.eq_ignore_ascii_case(&name))
                {
                    return Err("Profile name is already in use".into());
                }
                let unique: BTreeSet<_> = enabled.iter().collect();
                if unique.len() != enabled.len() {
                    return Err("Duplicate addon selection".into());
                }
                doc.name = name;
                doc.width = width;
                doc.height = height;
                doc.ui_scale = ui_scale;
                doc.profile.addons = enabled
                    .iter()
                    .map(|n| {
                        current
                            .available_addons
                            .iter()
                            .find(|a| a.name == *n)
                            .cloned()
                            .ok_or("Addon is not in this profile catalog")
                    })
                    .collect::<Result<_, _>>()?;
                // Renaming would split immutable native history. Use Copy for a new name.
                if doc.name != current.document.name {
                    return Err(
                        "Use Copy profile to create a new name; version history keeps its name"
                            .into(),
                    );
                }
            }
            Operation::CopyProfile { name, .. } => {
                if views
                    .iter()
                    .any(|v| v.document.name.eq_ignore_ascii_case(&name))
                {
                    return Err("Profile name is already in use".into());
                }
                doc.name = name;
            }
            Operation::RestoreProfile { version, .. } => {
                if !current.history.contains(&version) {
                    return Err("Restore version belongs to another profile".into());
                }
                doc = self.profile(&version)?;
            }
        }
        if self.scan()?.ids.len() >= PROFILE_CAPACITY {
            return Err("Player workspace profile capacity reached".into());
        }
        doc.save_new(&self.root)?;
        self.snapshot()
    }
}
