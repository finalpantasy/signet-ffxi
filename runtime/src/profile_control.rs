//! Profile controls are applied on the host owner thread. Effects are captured only.
use crate::profile_addons::{Control, ProfileHost};
use std::time::Duration;
use windower_rs::{
    addon_control::{Command, MAX_REQUESTS, REQUEST_LIMIT, Request},
    addon_profile::AddonProfile,
    commands::{Backend, Input},
};

pub struct Controller {
    host: ProfileHost,
    process_id: u32,
    nonce: String,
    owner: std::thread::ThreadId,
    next_sequence: u64,
    closed: bool,
    captured: Vec<serde_json::Value>,
    rejected: u64,
    thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl Controller {
    pub fn new(profile: AddonProfile, process_id: u32, nonce: String) -> Result<Self, String> {
        if process_id == 0 || !windower_rs::addon_control::valid_nonce(&nonce) {
            return Err("Invalid profile control instance".into());
        }
        Ok(Self {
            host: ProfileHost::new(profile)?,
            process_id,
            nonce,
            owner: std::thread::current().id(),
            next_sequence: 1,
            closed: false,
            captured: Vec::new(),
            rejected: 0,
            thread_bound: std::marker::PhantomData,
        })
    }
    pub fn next_sequence(&self) -> u64 {
        self.next_sequence
    }
    fn check_thread(&self) -> Result<(), String> {
        if std::thread::current().id() != self.owner {
            return Err("Profile control owner thread changed".into());
        }
        Ok(())
    }
    pub fn handle(&mut self, bytes: &[u8]) -> Result<serde_json::Value, String> {
        use sha2::{Digest, Sha256};
        self.check_thread()?;
        if self.next_sequence > MAX_REQUESTS {
            return Err("Profile control request limit reached".into());
        }
        let sequence = self.next_sequence;
        let result = if bytes.len() as u64 > REQUEST_LIMIT {
            Err("Oversized profile control request".into())
        } else {
            serde_json::from_slice::<Request>(bytes)
                .map_err(|_| "Invalid profile control request".to_string())
                .and_then(|request| {
                    request.validate(self.process_id, &self.nonce, sequence)?;
                    if self.closed {
                        return Err("Profile control is shut down".into());
                    }
                    match request.command {
                        Command::Status {} => Ok(()),
                        Command::Load { name, sha256 } => {
                            self.host.apply_pinned(Control::Load { name }, &sha256)
                        }
                        Command::Reload { name, sha256 } => {
                            self.host.apply_pinned(Control::Reload { name }, &sha256)
                        }
                        Command::Unload { name } => self.host.apply(Control::Unload { name }),
                        Command::Shutdown {} => {
                            self.host.shutdown();
                            self.closed = true;
                            Ok(())
                        }
                    }
                })
        };
        self.next_sequence += 1;
        if result.is_err() {
            self.rejected += 1;
        }
        Ok(
            serde_json::json!({"schema_version":1,"process_id":self.process_id,"nonce":self.nonce,
            "sequence":sequence,"request_sha256":format!("{:x}",Sha256::digest(bytes)),
            "ok":result.is_ok(),"error":result.err(),"status":self.host.status(),
            "closed":self.closed,"captured_effects":self.captured,"game_commands_sent":0}),
        )
    }
    pub fn tick(&mut self, now: Duration) -> Result<(), String> {
        self.check_thread()?;
        struct Capture<'a>(&'a mut Vec<serde_json::Value>);
        impl Capture<'_> {
            fn push(&mut self, value: serde_json::Value) -> Result<(), String> {
                let size = serde_json::to_vec(&value).map_err(|e| e.to_string())?.len();
                let used = self
                    .0
                    .iter()
                    .map(|v| serde_json::to_vec(v).map(|b| b.len()).unwrap_or(32769))
                    .sum::<usize>();
                if self.0.len() >= 32 || used + size > 32768 {
                    return Err("Profile control capture limit reached".into());
                }
                self.0.push(value);
                Ok(())
            }
        }
        impl Backend for Capture<'_> {
            fn input(&mut self, input: &Input) -> Result<(), String> {
                if self.0.len() >= 32 || input.bytes.len() > 4096 {
                    return Err("Profile control capture limit reached".into());
                }
                self.push(serde_json::json!({"origin":input.origin,"mode":input.mode,"bytes":input.bytes}))
            }
            fn chat(&mut self, mode: u8, bytes: &[u8]) -> Result<(), String> {
                if self.0.len() >= 32 || bytes.len() > 4096 {
                    return Err("Profile control capture limit reached".into());
                }
                self.push(serde_json::json!({"kind":"chat","mode":mode,"bytes":bytes}))
            }
        }
        self.host.tick(now, &mut Capture(&mut self.captured))
    }
    pub fn summary(&self) -> serde_json::Value {
        serde_json::json!({"status":self.host.status(),"closed":self.closed,
            "requests_processed":self.next_sequence-1,"rejected_requests":self.rejected,
            "captured_effects":self.captured,"game_commands_sent":0})
    }
}

#[cfg(all(windows, target_pointer_width = "32", feature = "game-thread-probe"))]
mod live {
    use super::*;
    use std::{
        fs::OpenOptions,
        io::{Read, Write},
        path::{Path, PathBuf},
        time::Instant,
    };
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{
            CreateEventW, GetCurrentProcessId, GetCurrentThreadId, SetEvent, WaitForSingleObject,
        },
    };

    pub struct Session {
        controller: Controller,
        directory: PathBuf,
        event: usize,
        response_event: usize,
        start: Instant,
        thread: u32,
    }
    fn publish(path: &Path, value: &serde_json::Value) -> Result<(), String> {
        let pending = path.with_extension("json.pending");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&pending)
            .map_err(|e| e.to_string())?;
        file.write_all(&serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        if path.exists() {
            return Err("Refusing existing control response".into());
        }
        std::fs::rename(pending, path).map_err(|e| e.to_string())
    }
    impl Session {
        pub fn new(lab: &windower_rs::runtime_options::LabTest) -> Result<Self, String> {
            let files = lab.file_profile.as_ref().ok_or("Missing control profile")?;
            files.validate()?;
            let profile = AddonProfile::read_verified(&files.path, &files.sha256)?;
            if profile.root.canonicalize().map_err(|e| e.to_string())?
                != files
                    .path
                    .parent()
                    .ok_or("Missing profile directory")?
                    .canonicalize()
                    .map_err(|e| e.to_string())?
            {
                return Err("Control profile root differs from its prepared root".into());
            }
            for (name, hash) in &files.sources {
                let (addon, leaf) = name.split_once('/').ok_or("Invalid prepared source")?;
                if profile.resolve_entry(addon, leaf)?.sha256 != *hash {
                    return Err("Prepared control source changed".into());
                }
            }
            let directory = lab
                .request
                .parent()
                .ok_or("Missing control directory")?
                .join("addon-control");
            std::fs::create_dir(&directory).map_err(|e| e.to_string())?;
            let pid = unsafe { GetCurrentProcessId() };
            let name = windower_rs::addon_control::event_name(pid, &lab.nonce);
            let controller = Controller::new(profile, pid, lab.nonce.clone())?;
            let make_event = |name: &str| {
                let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
                let h = unsafe { CreateEventW(std::ptr::null(), 0, 0, wide.as_ptr()) };
                if h.is_null() {
                    Err("Cannot create addon control event".to_string())
                } else {
                    Ok(h as usize)
                }
            };
            let event = make_event(&name)?;
            let response_event = match make_event(&(name.clone() + ".reply")) {
                Ok(h) => h,
                Err(e) => {
                    unsafe {
                        CloseHandle(event as _);
                    };
                    return Err(e);
                }
            };
            let session = Self {
                controller,
                directory,
                event,
                response_event,
                start: Instant::now(),
                thread: unsafe { GetCurrentThreadId() },
            };
            publish(
                &session.directory.join("ready.json"),
                &serde_json::json!({"schema_version":1,
                "process_id":pid,"nonce":lab.nonce,"directory":session.directory,"event":name,
                "owner_thread_id":session.thread,"profile_sha256":files.sha256,"effect_policy":"capture_only"}),
            )?;
            Ok(session)
        }
        pub fn tick(&mut self) -> Result<Option<serde_json::Value>, String> {
            if unsafe { GetCurrentThreadId() } != self.thread {
                return Err("Control draw thread changed".into());
            }
            if self.start.elapsed() > Duration::from_secs(120) {
                self.controller.host.shutdown();
                return Err("Profile control session deadline expired".into());
            }
            let now = self.start.elapsed();
            self.controller.tick(now)?;
            let wait = unsafe { WaitForSingleObject(self.event as _, 0) };
            if wait == 0 {
                let sequence = self.controller.next_sequence();
                let path = self.directory.join(format!("request-{sequence:06}.json"));
                let mut bytes = Vec::new();
                let file = match std::fs::File::open(path) {
                    Ok(file) => file,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                    Err(e) => return Err(e.to_string()),
                };
                file.take(REQUEST_LIMIT + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                let mut reply = self.controller.handle(&bytes)?;
                reply["owner_thread_id"] = self.thread.into();
                publish(
                    &self.directory.join(format!("response-{sequence:06}.json")),
                    &reply,
                )?;
                if unsafe { SetEvent(self.response_event as _) } == 0 {
                    return Err("Cannot signal control response".into());
                }
            } else if wait != 258 {
                return Err("Control event wait failed".into());
            }
            if self.controller.closed {
                let before = self.controller.captured.len();
                self.controller.tick(now + Duration::from_secs(60))?;
                if self.controller.captured.len() != before {
                    return Err("Effects survived profile shutdown".into());
                }
                let mut report = self.controller.summary();
                report["owner_thread_id"] = self.thread.into();
                report["clock_after_shutdown_advanced_without_effects"] = true.into();
                publish(&self.directory.join("closed.json"), &report)?;
                return Ok(Some(serde_json::json!({"profile_control":report})));
            }
            Ok(None)
        }
    }
    impl Drop for Session {
        fn drop(&mut self) {
            self.controller.host.shutdown();
            unsafe {
                CloseHandle(self.event as _);
                CloseHandle(self.response_event as _);
            }
        }
    }
}
#[cfg(all(windows, target_pointer_width = "32", feature = "game-thread-probe"))]
pub use live::Session;

#[cfg(test)]
mod tests {
    use super::*;
    use windower_rs::{
        platform::Ecosystem,
        runtime_options::{AddonSpec, LuaEngine},
    };
    const SOURCE: &str = "ffxi.events.on('load',function() ffxi.commands.queue('wait 30;input /echo pending');coroutine.schedule(function() ffxi.commands.input('/echo delayed') end,30) end)";
    fn controller() -> Controller {
        let root = std::env::temp_dir().join(format!(
            "rust-profile-controls-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for name in ["selected", "neighbor"] {
            let dir = root.join("addons").join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("{name}.lua")), SOURCE).unwrap();
        }
        Controller::new(
            AddonProfile {
                schema_version: 1,
                luajit_provider: None,
                root,
                addons: ["selected", "neighbor"]
                    .into_iter()
                    .map(|name| AddonSpec {
                        name: name.into(),
                        ecosystem: Ecosystem::Native,
                        engine: LuaEngine::Lua51,
                    })
                    .collect(),
            },
            17,
            "a".repeat(32),
        )
        .unwrap()
    }
    fn send(c: &mut Controller, command: Command) -> serde_json::Value {
        let request = Request {
            schema_version: 1,
            process_id: 17,
            nonce: "a".repeat(32),
            sequence: c.next_sequence(),
            command,
        };
        c.handle(&serde_json::to_vec(&request).unwrap()).unwrap()
    }
    #[test]
    fn controls_pin_sources_preserve_neighbors_and_cancel_on_shutdown() {
        let mut c = controller();
        for name in ["selected", "neighbor"] {
            use sha2::{Digest, Sha256};
            let sha256 = format!("{:x}", Sha256::digest(SOURCE.as_bytes()));
            assert_eq!(
                send(
                    &mut c,
                    Command::Load {
                        name: name.into(),
                        sha256
                    }
                )["ok"],
                true
            );
        }
        c.tick(Duration::ZERO).unwrap();
        let before = serde_json::to_value(c.host.status()).unwrap();
        assert_eq!(
            send(
                &mut c,
                Command::Reload {
                    name: "selected".into(),
                    sha256: "0".repeat(64)
                }
            )["ok"],
            false
        );
        assert_eq!(serde_json::to_value(c.host.status()).unwrap(), before);
        assert_eq!(
            send(
                &mut c,
                Command::Unload {
                    name: "selected".into()
                }
            )["ok"],
            true
        );
        assert_eq!(c.host.status().loaded_sources.len(), 1);
        assert!(c.host.status().loaded_sources.contains_key("neighbor"));
        assert_eq!(send(&mut c, Command::Shutdown {})["ok"], true);
        c.tick(Duration::from_secs(60)).unwrap();
        assert_eq!(c.host.status().pending_batches, 0);
        assert!(c.captured.is_empty());
        assert_eq!(c.host.status().stats.cancelled_batches, 2);
        assert_eq!(c.host.status().stats.cancelled_schedules, 2);
        assert_eq!(send(&mut c, Command::Status {})["ok"], false);
    }
    #[test]
    fn malformed_wrong_instance_and_replayed_controls_do_not_mutate_host() {
        let mut c = controller();
        for bytes in [
            b"{invalid".to_vec(),
            vec![b' '; REQUEST_LIMIT as usize + 1],
            serde_json::to_vec(&Request {
                schema_version: 1,
                process_id: 18,
                nonce: "a".repeat(32),
                sequence: 3,
                command: Command::Shutdown {},
            })
            .unwrap(),
            serde_json::to_vec(&Request {
                schema_version: 1,
                process_id: 17,
                nonce: "a".repeat(32),
                sequence: 1,
                command: Command::Shutdown {},
            })
            .unwrap(),
        ] {
            assert_eq!(c.handle(&bytes).unwrap()["ok"], false);
            assert!(!c.closed);
            assert!(c.host.status().loaded_sources.is_empty());
        }
        assert_eq!(send(&mut c, Command::Status {})["ok"], true);
        assert_eq!(
            send(
                &mut c,
                Command::Load {
                    name: "unselected".into(),
                    sha256: "a".repeat(64)
                }
            )["ok"],
            false
        );
    }
}
