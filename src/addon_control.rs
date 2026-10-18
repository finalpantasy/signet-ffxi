//! Typed local profile lifecycle protocol; never accepts Lua or game commands.
use serde::{Deserialize, Serialize};

pub const REQUEST_LIMIT: u64 = 16384;
pub const MAX_REQUESTS: u64 = 128;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Status {},
    Load { name: String, sha256: String },
    Reload { name: String, sha256: String },
    Unload { name: String },
    Shutdown {},
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema_version: u32,
    pub process_id: u32,
    pub nonce: String,
    pub sequence: u64,
    pub command: Command,
}

pub fn event_name(process_id: u32, nonce: &str) -> String {
    format!("WindowerRust.AddonControl.{process_id}.{nonce}")
}

pub fn valid_nonce(nonce: &str) -> bool {
    nonce.len() == 32
        && nonce
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

impl Request {
    pub fn validate(&self, process_id: u32, nonce: &str, sequence: u64) -> Result<(), String> {
        if self.schema_version != 1
            || process_id == 0
            || self.process_id != process_id
            || !valid_nonce(nonce)
            || self.nonce != nonce
            || self.sequence != sequence
            || !(1..=MAX_REQUESTS).contains(&sequence)
        {
            return Err("Addon control identity or sequence mismatch".into());
        }
        match &self.command {
            Command::Load { name, sha256 } | Command::Reload { name, sha256 } => {
                if sha256.len() != 64
                    || !sha256
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                {
                    return Err("Addon control requires a lowercase source digest".into());
                }
                validate_name(name)
            }
            Command::Unload { name } => validate_name(name),
            _ => Ok(()),
        }
    }
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 256
        || name == "."
        || name == ".."
        || name.trim() != name
        || name
            .chars()
            .any(|c| c.is_control() || "/\\:<>\"|?*".contains(c))
    {
        return Err("Invalid addon selection name".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> Request {
        Request {
            schema_version: 1,
            process_id: 17,
            nonce: "a".repeat(32),
            sequence: 1,
            command: Command::Status {},
        }
    }
    #[test]
    fn control_identity_replay_and_bounds_are_strict() {
        let r = request();
        assert!(r.validate(17, &"a".repeat(32), 1).is_ok());
        for (pid, nonce, sequence) in [
            (18, "a".repeat(32), 1),
            (17, "b".repeat(32), 1),
            (17, "a".repeat(32), 2),
            (0, "a".repeat(32), 1),
            (17, "a".repeat(32), 129),
        ] {
            assert!(r.validate(pid, &nonce, sequence).is_err());
        }
        let mut r = r;
        r.schema_version = 2;
        assert!(r.validate(17, &"a".repeat(32), 1).is_err());
    }
    #[test]
    fn control_refuses_eval_paths_unknown_fields_and_bad_pins() {
        for command in [
            serde_json::json!({"action":"eval","lua":"return 1"}),
            serde_json::json!({"action":"status","extra":true}),
            serde_json::json!({"action":"shutdown","name":"x"}),
        ] {
            assert!(serde_json::from_value::<Command>(command).is_err());
        }
        for (name, sha256) in [
            ("../x", "a".repeat(64)),
            ("x", "A".repeat(64)),
            ("x", "x".repeat(64)),
        ] {
            let mut r = request();
            r.command = Command::Load {
                name: name.into(),
                sha256,
            };
            assert!(r.validate(17, &"a".repeat(32), 1).is_err());
        }
    }
}
