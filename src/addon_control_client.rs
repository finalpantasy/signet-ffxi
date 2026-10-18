//! Windows launcher side of the explicitly enabled local lifecycle mailbox.
use crate::addon_control::{
    Command, MAX_REQUESTS, REQUEST_LIMIT, Request, event_name, valid_nonce,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::CloseHandle,
    System::Threading::{OpenEventW, SetEvent, WaitForSingleObject},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Connection {
    schema_version: u32,
    process_id: u32,
    nonce: String,
    directory: PathBuf,
    event: String,
    owner_thread_id: u32,
    profile_sha256: String,
    effect_policy: String,
}
struct Handle(windows_sys::Win32::Foundation::HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Expected a regular control file".into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("Control file exceeds limit".into());
    }
    Ok(bytes)
}
fn connect(ready: &Path) -> Result<Connection, String> {
    let connection: Connection = serde_json::from_slice(&bounded(ready, REQUEST_LIMIT)?)
        .map_err(|_| "Invalid control connection")?;
    if connection.schema_version != 1
        || connection.process_id == 0
        || !valid_nonce(&connection.nonce)
        || connection.owner_thread_id == 0
        || connection.effect_policy != "capture_only"
        || connection.profile_sha256.len() != 64
        || connection.event != event_name(connection.process_id, &connection.nonce)
        || ready.canonicalize().map_err(|e| e.to_string())?.parent()
            != Some(
                connection
                    .directory
                    .canonicalize()
                    .map_err(|e| e.to_string())?
                    .as_path(),
            )
    {
        return Err("Control connection identity/path mismatch".into());
    }
    if connection.directory.join("closed.json").exists() {
        return Err("Control session is already closed".into());
    }
    Ok(connection)
}
/// Send one typed command to the next unused consecutive mailbox slot.
/// Races with another sender fail through exclusive publication; never retry a command.
pub fn send_next(ready: &Path, command: Command) -> Result<serde_json::Value, String> {
    let connection = connect(ready)?;
    let sequence = next_sequence(&connection.directory)?;
    send_connection(connection, sequence, command)
}
fn next_sequence(directory: &Path) -> Result<u64, String> {
    let mut next = None;
    for sequence in 1..=MAX_REQUESTS {
        let response = directory
            .join(format!("response-{sequence:06}.json"))
            .exists();
        let request = directory
            .join(format!("request-{sequence:06}.json"))
            .exists()
            || directory
                .join(format!("request-{sequence:06}.json.pending"))
                .exists();
        if next.is_some() && (response || request) {
            return Err("Control mailbox has a sequence gap".into());
        }
        if !response && next.is_none() {
            if request {
                return Err(
                    "A control request is already pending; inspect its result before continuing"
                        .into(),
                );
            }
            next = Some(sequence);
        }
    }
    next.ok_or("Control request capacity reached".into())
}
pub fn send(ready: &Path, sequence: u64, command_path: &Path) -> Result<serde_json::Value, String> {
    let connection = connect(ready)?;
    let command: Command = serde_json::from_slice(&bounded(command_path, REQUEST_LIMIT)?)
        .map_err(|_| "Invalid typed addon control")?;
    send_connection(connection, sequence, command)
}
fn send_connection(
    connection: Connection,
    sequence: u64,
    command: Command,
) -> Result<serde_json::Value, String> {
    let request = Request {
        schema_version: 1,
        process_id: connection.process_id,
        nonce: connection.nonce.clone(),
        sequence,
        command,
    };
    request.validate(connection.process_id, &connection.nonce, sequence)?;
    let open = |name: &str, access| {
        let wide: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
        let handle = unsafe { OpenEventW(access, 0, wide.as_ptr()) };
        if handle.is_null() {
            Err("Control runtime is not available".to_string())
        } else {
            Ok(Handle(handle))
        }
    };
    let event = open(&connection.event, 2)?;
    let response_event = open(&(connection.event.clone() + ".reply"), 0x00100000)?;
    let response_path = connection
        .directory
        .join(format!("response-{sequence:06}.json"));
    if response_path.exists() {
        return Err("Control sequence already has a response".into());
    }
    if sequence > 1
        && !connection
            .directory
            .join(format!("response-{:06}.json", sequence - 1))
            .exists()
    {
        return Err("Prior control sequence has not completed".into());
    }
    let path = connection
        .directory
        .join(format!("request-{sequence:06}.json"));
    let pending = path.with_extension("json.pending");
    let bytes = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    if path.exists() {
        return Err("Control sequence already published".into());
    }
    std::fs::rename(pending, path).map_err(|e| e.to_string())?;
    if unsafe { SetEvent(event.0) } == 0 {
        return Err("Cannot signal addon control".into());
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while !response_path.exists() {
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as u32;
        if remaining == 0 || unsafe { WaitForSingleObject(response_event.0, remaining) } != 0 {
            return Err("Addon control response deadline expired".into());
        }
    }
    let response: serde_json::Value = serde_json::from_slice(&bounded(&response_path, 262144)?)
        .map_err(|_| "Invalid addon control response")?;
    if response["process_id"] != connection.process_id
        || response["schema_version"] != 1
        || response["game_commands_sent"] != 0
        || response["ok"].as_bool().is_none()
        || response["status"]["loaded_sources"].as_object().is_none()
        || response["nonce"] != connection.nonce
        || response["sequence"] != sequence
        || response["owner_thread_id"] != connection.owner_thread_id
        || response["request_sha256"] != format!("{:x}", Sha256::digest(&bytes))
    {
        return Err("Addon control response identity mismatch".into());
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_sequence_refuses_pending_gaps_and_exhaustion() {
        let directory = std::env::temp_dir().join(format!(
            "rust-control-sequence-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        assert_eq!(next_sequence(&directory).unwrap(), 1);
        std::fs::write(directory.join("request-000001.json.pending"), b"pending").unwrap();
        assert!(next_sequence(&directory).unwrap_err().contains("pending"));
        // Preserve evidence; completing the slot permits the next one without deleting anything.
        std::fs::write(directory.join("response-000001.json"), b"completed").unwrap();
        assert_eq!(next_sequence(&directory).unwrap(), 2);
        std::fs::write(directory.join("response-000003.json"), b"gap").unwrap();
        assert!(next_sequence(&directory).unwrap_err().contains("gap"));
        for n in 2..=MAX_REQUESTS {
            std::fs::write(
                directory.join(format!("response-{n:06}.json")),
                b"completed",
            )
            .unwrap();
        }
        assert!(next_sequence(&directory).unwrap_err().contains("capacity"));
    }
}
