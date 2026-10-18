//! Pinned loopback loader configuration. Credential values never enter reports
//! or the command line: the loader receives only its existing private JSON path.
use crate::runtime_options::NativeLibrary;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalClient {
    pub loader: NativeLibrary,
    pub login_json: PathBuf,
}
impl LocalClient {
    pub fn verify(&self) -> Result<(), String> {
        self.loader.validate()?;
        if !self.login_json.is_absolute() {
            return Err("Local login configuration requires an absolute path".into());
        }
        let bytes = std::fs::read(&self.loader.path).map_err(|e| e.to_string())?;
        if !format!("{:x}", Sha256::digest(&bytes)).eq_ignore_ascii_case(&self.loader.sha256) {
            return Err("Local loader changed after preparation".into());
        }
        let login = std::fs::read(&self.login_json)
            .map_err(|_| "Cannot read the private local login configuration")?;
        validate_login(&login)
    }
    pub fn arguments(&self) -> Vec<OsString> {
        vec!["--json".into(), self.login_json.as_os_str().into()]
    }
}
pub fn validate_login(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > 65536 {
        return Err("Private login configuration exceeds limits".into());
    }
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|_| "Private login configuration is not valid JSON")?;
    if !matches!(
        value.get("server").and_then(|v| v.as_str()),
        Some("127.0.0.1" | "127.0.0.2")
    ) {
        return Err(
            "Rust local trial only admits the explicit primary or isolated loopback server".into(),
        );
    }
    for key in ["username", "password"] {
        if value
            .get(key)
            .and_then(|v| v.as_str())
            .is_none_or(|s| s.is_empty() || s.contains('\0'))
        {
            return Err("Private login configuration lacks usable credentials".into());
        }
    }
    for key in ["dataport", "viewport", "authport", "profileport"] {
        if let Some(port) = value.get(key)
            && port.as_u64().is_none_or(|p| !(1..=65535).contains(&p))
        {
            return Err("Private login configuration has an invalid local port".into());
        }
    }
    Ok(())
}
pub fn read(path: &Path) -> Result<LocalClient, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() > 65536 {
        return Err("Local launch definition exceeds limits".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "Invalid local launch definition".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn isolated_loopback_is_explicit_and_other_addresses_stay_refused() {
        let mut value = serde_json::json!({"server":"127.0.0.2","username":"fixture-user","password":"fixture-secret","profileport":51220});
        validate_login(&serde_json::to_vec(&value).unwrap()).unwrap();
        for address in [
            "127.0.0.3",
            "127.1.0.2",
            "127.0.0.2.example",
            "::1",
            "0.0.0.0",
        ] {
            value["server"] = address.into();
            assert!(validate_login(&serde_json::to_vec(&value).unwrap()).is_err());
        }
    }
    #[test]
    fn login_is_loopback_only_and_errors_do_not_echo_credentials() {
        let mut value = serde_json::json!({"server":"127.0.0.1","username":"fixture-user","password":"fixture-secret"});
        validate_login(&serde_json::to_vec(&value).unwrap()).unwrap();
        for server in ["localhost", "192.0.2.1", "127.0.0.1.example"] {
            value["server"] = server.into();
            let error = validate_login(&serde_json::to_vec(&value).unwrap()).unwrap_err();
            assert!(!error.contains("fixture-secret"));
        }
        value["server"] = "127.0.0.1".into();
        value["authport"] = 0.into();
        assert!(validate_login(&serde_json::to_vec(&value).unwrap()).is_err());
        let local = LocalClient {
            loader: NativeLibrary {
                path: PathBuf::from("C:/fixture/loader.exe"),
                sha256: "0".repeat(64),
            },
            login_json: PathBuf::from("C:/private/login.json"),
        };
        assert_eq!(
            local.arguments(),
            vec![
                OsString::from("--json"),
                OsString::from("C:/private/login.json")
            ]
        );
    }
}
