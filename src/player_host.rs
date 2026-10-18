//! Owns only the capture-only Lua host created here; no PID attachment or shell.
use crate::{launcher_document::Document, packages::digest, runtime_options::NativeLibrary};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};
pub struct Session {
    child: Child,
    input: ChildStdin,
    replies: Receiver<Result<Value, String>>,
    pub profile_id: String,
    pub launched: Document,
    pub latest: Value,
}
impl Session {
    pub fn start(
        host: &NativeLibrary,
        profile_id: String,
        document: Document,
        directory: &Path,
    ) -> Result<Self, String> {
        host.validate()?;
        let bytes = std::fs::read(&host.path).map_err(|e| e.to_string())?;
        if digest(&bytes) != host.sha256 {
            return Err("Offline host changed after selection".into());
        }
        let expected_sources = document.validate_files()?;
        std::fs::create_dir(directory).map_err(|e| e.to_string())?;
        let path = directory.join("addons.json");
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&document.profile).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let mut cmd = Command::new(&host.path);
        cmd.arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        let input = child.stdin.take().ok_or("Missing owned host input")?;
        let output = child.stdout.take().ok_or("Missing owned host output")?;
        let (tx, rx) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(output);
            loop {
                let mut bytes = Vec::new();
                let read = reader.by_ref().take(1048577).read_until(b'\n', &mut bytes);
                let message = match read {
                    Ok(0) => break,
                    Ok(n) if n <= 1048576 => {
                        serde_json::from_slice(&bytes).map_err(|_| "Invalid host reply".into())
                    }
                    _ => Err("Host reply exceeded limits or failed".into()),
                };
                if tx.send(message).is_err() {
                    break;
                }
            }
        });
        let mut session = Self {
            child,
            input,
            replies: rx,
            profile_id,
            launched: document,
            latest: Value::Null,
        };
        session.latest = session.receive()?;
        if session.latest["stage"] != "ready" || session.latest["game_commands_sent"] != 0 {
            return Err("Owned offline host did not become ready".into());
        }
        for (name, expected) in expected_sources {
            let loaded = &session.latest["status"]["loaded_sources"][name.to_lowercase()];
            if !loaded.is_null() && loaded.as_str() != Some(expected.as_str()) {
                return Err("Addon bytes changed during runtime startup".into());
            }
        }
        Ok(session)
    }
    fn receive(&self) -> Result<Value, String> {
        self.replies
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| "Owned runtime timed out or exited; stop this session before retrying")?
    }
    pub fn is_running(&mut self) -> Result<bool, String> {
        self.child
            .try_wait()
            .map(|status| status.is_none())
            .map_err(|e| e.to_string())
    }
    pub fn shutdown(&mut self) -> Result<Value, String> {
        let mut reply = self.control("shutdown", None)?;
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(status) = self.child.try_wait().map_err(|e| e.to_string())? {
                if !status.success() {
                    return Err(format!("Owned runtime shutdown failed: {status}"));
                }
                reply["exit_code"] = status.code().into();
                return Ok(reply);
            }
            if Instant::now() >= deadline {
                return Err("Owned runtime did not exit after shutdown".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    pub fn control(&mut self, action: &str, name: Option<&str>) -> Result<Value, String> {
        if self.child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err("Owned runtime has exited".into());
        }
        if !["status", "load", "reload", "unload", "shutdown"].contains(&action) {
            return Err("Unsupported runtime operation".into());
        }
        if ["load", "reload", "unload"].contains(&action)
            && !self
                .launched
                .profile
                .addons
                .iter()
                .any(|a| Some(a.name.as_str()) == name)
        {
            return Err("Addon is not selected in the running session".into());
        }
        let mut request = json!({"action":action});
        if let Some(name) = name {
            request["name"] = name.into();
        }
        writeln!(self.input, "{request}")
            .and_then(|_| self.input.flush())
            .map_err(|e| e.to_string())?;
        self.latest = self.receive()?;
        if self.latest["game_commands_sent"] != 0 || !self.latest["ok"].is_boolean() {
            return Err("Runtime response failed offline contract".into());
        }
        Ok(self.view())
    }
    pub fn view(&self) -> Value {
        json!({"mode":"offline_capture","process_id":self.child.id(),"profile_id":self.profile_id,"document":self.launched,"reply":self.latest,"game_started":false})
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
