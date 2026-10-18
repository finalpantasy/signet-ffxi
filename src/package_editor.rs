//! Local package panel operations. The store is fixed inside the chosen workspace.
use crate::packages::{Manifest, Store, resolve};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Inspect,
    Prepare,
    Stage { manifest: PathBuf, source: PathBuf },
    Check { slots: Vec<String> },
    Activate { slots: Vec<String> },
    Rollback { generation: u64 },
}
pub fn execute(workspace: &Path, request: Request) -> Result<serde_json::Value, String> {
    if !workspace.is_absolute() || !workspace.is_dir() {
        return Err("Choose an existing absolute workspace".into());
    }
    let path = workspace.join("packages");
    // Reject an existing redirected store before Store::open can follow it.
    if let Ok(meta) = std::fs::symlink_metadata(&path) {
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err("Redirected workspace package store refused".into());
            }
        }
        if meta.file_type().is_symlink() {
            return Err("Redirected workspace package store refused".into());
        }
    }
    let store = Store::open(&path)?;
    match request {
        Request::Prepare => {
            let parent = workspace.join("runtime-versions");
            match std::fs::create_dir(&parent) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.to_string()),
            }
            let meta = std::fs::symlink_metadata(&parent).map_err(|e| e.to_string())?;
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x400 != 0 {
                    return Err("Redirected runtime directory refused".into());
                }
            }
            if !meta.is_dir() || meta.file_type().is_symlink() {
                return Err("Invalid runtime directory".into());
            }
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos();
            let mount = store.prepare_runtime(
                &parent.join(format!("prepared-{}-{stamp}", std::process::id())),
            )?;
            Ok(serde_json::json!({"action":"prepare","mount":mount}))
        }
        Request::Inspect => {
            Ok(serde_json::json!({"action":"inspect","current":store.current()?,"store":path}))
        }
        Request::Stage { manifest, source } => {
            if !manifest.is_absolute() || !source.is_absolute() {
                return Err("Choose absolute local manifest and source paths".into());
            }
            let receipt = store.stage(&Manifest::read(&manifest)?, &source)?;
            Ok(serde_json::json!({"action":"stage","receipt":receipt,"current":store.current()?}))
        }
        Request::Check { slots } => {
            if slots.is_empty() || slots.len() > 256 {
                return Err("Select between 1 and 256 staged slots".into());
            }
            let manifests = slots
                .iter()
                .map(|slot| store.verify_slot(slot).map(|r| r.manifest))
                .collect::<Result<Vec<_>, _>>()?;
            let plan = resolve(
                &manifests,
                &manifests
                    .iter()
                    .map(|m| (m.id.clone(), m.version.clone()))
                    .collect::<Vec<_>>(),
            )?;
            Ok(serde_json::json!({"action":"check","plan":plan,"current":store.current()?}))
        }
        Request::Activate { slots } => {
            if slots.is_empty() {
                return Err("Select the complete nonempty set of staged slots".into());
            }
            Ok(serde_json::json!({"action":"activate","current":store.activate(&slots)?}))
        }
        Request::Rollback { generation } => {
            Ok(serde_json::json!({"action":"rollback","current":store.rollback(generation)?}))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packages::digest;
    #[test]
    fn bad_stage_and_selection_preserve_active_generation() {
        let root = std::env::temp_dir().join(format!(
            "package-panel-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("addon.lua"), b"return 1").unwrap();
        let manifest = root.join("manifest.json");
        std::fs::write(&manifest,serde_json::to_vec(&serde_json::json!({"schema_version":1,"id":"example","version":"1.0.0","files":{"addon.lua":digest(b"return 1")}})).unwrap()).unwrap();
        let staged = execute(
            &root,
            Request::Stage {
                manifest: manifest.clone(),
                source: source.clone(),
            },
        )
        .unwrap();
        let slots = vec![staged["receipt"]["slot"].as_str().unwrap().to_string()];
        execute(
            &root,
            Request::Check {
                slots: slots.clone(),
            },
        )
        .unwrap();
        execute(&root, Request::Activate { slots }).unwrap();
        let pointer = std::fs::read(root.join("packages/CURRENT")).unwrap();
        std::fs::write(source.join("addon.lua"), b"tampered").unwrap();
        assert!(execute(&root, Request::Stage { manifest, source }).is_err());
        assert!(execute(&root, Request::Activate { slots: vec![] }).is_err());
        assert!(
            execute(
                &root,
                Request::Check {
                    slots: vec!["../foreign".into()]
                }
            )
            .is_err()
        );
        assert!(execute(&root, Request::Rollback { generation: 999 }).is_err());
        assert_eq!(
            std::fs::read(root.join("packages/CURRENT")).unwrap(),
            pointer
        );
    }
}
