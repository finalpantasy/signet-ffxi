//! Client updating for retail FINAL FANTASY XI, through `signet-patch`.
//!
//! Every network call and file write runs on a worker thread; the UI gets the whole
//! state as a JSON event each time it changes (throttled while progress moves).
//! Nothing here runs without a click, except the one small version check at startup.
//!
//! Test seams (environment): `SIGNET_PATCH_SERVER=127.0.0.1:PORT` (loopback only),
//! `SIGNET_PATCH_STAGING`, `SIGNET_PATCH_BACKUPS`. An invalid server value points at a
//! closed local port, so a typo never reaches the real server.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};
use signet_patch::{ApplyOptions, Cancel, LineageVerdict, LiveServer, PatchError, PlanOptions, Progress};

const CHECK_TIMEOUT: Duration = Duration::from_secs(6);
const EMIT_EVERY: Duration = Duration::from_millis(80);
const UNREACHABLE: &str = "Couldn't reach the update server.";
const CLOSE_FIRST: &str = "Close PlayOnline and FFXI first.";

pub struct Config {
    pub install: PathBuf,
    pub staging: PathBuf,
    pub backups: PathBuf,
    pub server: LiveServer,
}

impl Config {
    pub fn from_env(install: PathBuf) -> Config {
        Self::from_lookup(install, |k| std::env::var(k).ok())
    }

    pub fn from_lookup(install: PathBuf, get: impl Fn(&str) -> Option<String>) -> Config {
        let mut server = LiveServer::retail_ffxi();
        if let Some(v) = get("SIGNET_PATCH_SERVER") {
            let parsed = v
                .rsplit_once(':')
                .filter(|(h, _)| matches!(*h, "127.0.0.1" | "localhost"))
                .and_then(|(h, p)| Some((h.to_string(), p.parse::<u16>().ok()?)));
            (server.host, server.port) = parsed.unwrap_or(("127.0.0.1".into(), 1));
        }
        let dir = |key: &str, default: PathBuf| get(key).map(PathBuf::from).unwrap_or(default);
        Config {
            install,
            staging: dir("SIGNET_PATCH_STAGING", signet_patch::default_staging()),
            backups: dir("SIGNET_PATCH_BACKUPS", signet_patch::default_backups()),
            server,
        }
    }
}

#[derive(Clone, Serialize)]
struct Check {
    /// idle, checking, current, available, offline
    status: &'static str,
    installed: Option<String>,
    server: Option<String>,
    message: Option<String>,
}

#[derive(Clone, Serialize)]
struct PlanInfo {
    files: usize,
    bytes: u64,
    version: String,
    /// Nothing to download and the file list already matches.
    current: bool,
}

#[derive(Clone, Default, Serialize)]
struct Prog {
    /// connect, scan, download, install
    stage: &'static str,
    scanned: usize,
    scan_total: usize,
    done_bytes: u64,
    total_bytes: u64,
    verified: usize,
    verify_total: usize,
    installed: usize,
    install_total: usize,
}

#[derive(Clone, Serialize)]
struct Done {
    version: String,
    installed: usize,
    downloaded_bytes: u64,
    backup: Option<String>,
}

#[derive(Clone, Serialize)]
struct Job {
    /// idle, planning, planned, applying, done, failed, cancelled
    phase: &'static str,
    plan: Option<PlanInfo>,
    progress: Prog,
    result: Option<Done>,
    error: Option<String>,
    cancelling: bool,
}

#[derive(Clone, Serialize)]
struct Restore {
    /// idle, running, done, failed
    phase: &'static str,
    done: usize,
    total: usize,
    restored: usize,
    removed: usize,
    partial: bool,
    error: Option<String>,
}

#[derive(Clone, Serialize)]
struct State {
    check: Check,
    job: Job,
    restore: Restore,
}

fn idle_job() -> Job {
    Job { phase: "idle", plan: None, progress: Prog::default(), result: None, error: None, cancelling: false }
}

fn idle_restore() -> Restore {
    Restore { phase: "idle", done: 0, total: 0, restored: 0, removed: 0, partial: false, error: None }
}

type Emitter = Box<dyn Fn(Value) + Send + Sync>;

pub struct Updater {
    cfg: Config,
    state: Mutex<State>,
    cancel: Mutex<Cancel>,
    busy: AtomicBool,
    emitter: OnceLock<Emitter>,
    last_emit: Mutex<Instant>,
}

/// Plain words for the UI. `during_install` marks a failure after files started to change.
fn friendly(e: &PatchError, during_install: bool) -> String {
    let text = match e {
        PatchError::Cancelled => "Cancelled.".to_string(),
        PatchError::Refused(m) if m.contains("close the game first") => CLOSE_FIRST.to_string(),
        PatchError::Refused(m) => m.clone(),
        PatchError::Failed(m) => {
            let l = m.to_ascii_lowercase();
            if ["cannot connect", "cannot look up", "patch server", "timed out"].iter().any(|k| l.contains(k)) {
                UNREACHABLE.to_string()
            } else {
                m.clone()
            }
        }
    };
    let mut text: String = text.chars().take(300).collect();
    if during_install && !matches!(e, PatchError::Cancelled) {
        text = format!("The update stopped part way. {text} Use Restore previous files to undo it.");
    }
    text
}

impl Updater {
    pub fn new(cfg: Config) -> Arc<Updater> {
        let installed = signet_patch::local_version(&cfg.install);
        Arc::new(Updater {
            state: Mutex::new(State {
                check: Check { status: "idle", installed, server: None, message: None },
                job: idle_job(),
                restore: idle_restore(),
            }),
            cfg,
            cancel: Mutex::new(Cancel::new()),
            busy: AtomicBool::new(false),
            emitter: OnceLock::new(),
            last_emit: Mutex::new(Instant::now() - EMIT_EVERY),
        })
    }

    pub fn set_emitter(&self, f: impl Fn(Value) + Send + Sync + 'static) {
        let _ = self.emitter.set(Box::new(f));
    }

    /// Whole state for the UI, with the newest unrestored backup.
    pub fn state(&self) -> Value {
        let mut v = serde_json::to_value(&*self.state.lock().unwrap()).unwrap();
        v["backup"] = signet_patch::latest_backup(&self.cfg.backups)
            .map(|p| json!({ "path": p, "name": p.file_name().map(|n| n.to_string_lossy().into_owned()) }))
            .unwrap_or(Value::Null);
        v
    }

    /// Image names of programs that stop an update (PlayOnline, FFXI).
    pub fn blocked(&self) -> Vec<String> {
        let mut names: Vec<String> = signet_patch::blockers(&self.cfg.install)
            .into_iter()
            .map(|b| b.image.replace('/', "\\").rsplit('\\').next().unwrap_or("").to_string())
            .collect();
        names.sort();
        names.dedup();
        names
    }

    pub fn backups_dir(&self) -> &Path {
        &self.cfg.backups
    }

    fn emit_now(&self) {
        *self.last_emit.lock().unwrap() = Instant::now();
        if let Some(f) = self.emitter.get() {
            f(self.state());
        }
    }

    /// Change the state; send it now when `force`, else at most every 80 ms.
    fn update(&self, force: bool, f: impl FnOnce(&mut State)) {
        f(&mut self.state.lock().unwrap());
        let due = self.last_emit.lock().unwrap().elapsed() >= EMIT_EVERY;
        if force || due {
            self.emit_now();
        }
    }

    fn start_job(&self) -> Result<Cancel, String> {
        if self.busy.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            return Err("An update step is already running.".into());
        }
        let fresh = Cancel::new();
        *self.cancel.lock().unwrap() = fresh.clone();
        Ok(fresh)
    }

    fn source(&self) -> LiveServer {
        self.cfg.server.clone()
    }

    /// One small request: the server's version against the install's patch.cfg.
    /// Runs on the calling thread and never raises an error: offline is a status.
    pub fn check(&self) -> Value {
        let installed = signet_patch::local_version(&self.cfg.install);
        self.update(true, |s| s.check = Check { status: "checking", installed: installed.clone(), server: None, message: None });
        let mut src = self.source();
        src.timeout = CHECK_TIMEOUT;
        let check = match signet_patch::check_version(&src) {
            Ok(v) => {
                let behind = match installed.as_deref() {
                    Some(i) => signet_patch::version_order(i, &v.version).is_lt(),
                    None => true,
                };
                Check { status: if behind { "available" } else { "current" }, installed, server: Some(v.version), message: None }
            }
            Err(_) => Check { status: "offline", installed, server: None, message: Some(UNREACHABLE.into()) },
        };
        self.update(true, |s| s.check = check);
        self.state()
    }

    /// Fetch the file list and compare it with the install. Writes nothing.
    pub fn plan(self: &Arc<Self>) -> Result<Value, String> {
        let cancel = self.start_job()?;
        self.update(true, |s| {
            s.job = idle_job();
            s.job.phase = "planning";
            s.job.progress.stage = "connect";
        });
        let me = self.clone();
        std::thread::spawn(move || {
            let r = signet_patch::plan(&PlanOptions::new(me.cfg.install.clone()), &me.source(), &|p| me.on_progress(p), &cancel);
            me.update(true, |s| match r {
                Ok(rep) => match rep.lineage {
                    LineageVerdict::Foreign(ref why) => {
                        s.job.phase = "failed";
                        s.job.error = Some(why.chars().take(400).collect());
                    }
                    _ => {
                        let current = rep.survey.work.is_empty() && !rep.catalog_differs();
                        s.job.phase = "planned";
                        s.job.plan = Some(PlanInfo { files: rep.survey.work.len(), bytes: rep.survey.bytes, version: rep.server.version.clone(), current });
                    }
                },
                Err(PatchError::Cancelled) => s.job = idle_job(),
                Err(e) => {
                    s.job.phase = "failed";
                    s.job.error = Some(friendly(&e, false));
                }
            });
            me.busy.store(false, Ordering::SeqCst);
        });
        Ok(self.state())
    }

    /// Download, verify and install. Refuses while PlayOnline or FFXI runs.
    pub fn apply(self: &Arc<Self>) -> Result<Value, String> {
        if !self.blocked().is_empty() {
            return Err(CLOSE_FIRST.into());
        }
        let cancel = self.start_job()?;
        self.update(true, |s| {
            let plan = s.job.plan.clone();
            s.job = idle_job();
            s.job.plan = plan;
            s.job.phase = "applying";
            s.job.progress.stage = "connect";
        });
        let me = self.clone();
        std::thread::spawn(move || {
            let mut opts = ApplyOptions::new(me.cfg.install.clone());
            opts.staging = me.cfg.staging.clone();
            opts.backups = me.cfg.backups.clone();
            opts.plan.save_catalog_in = Some(me.cfg.staging.clone());
            let r = signet_patch::apply(&opts, &me.source(), &|p| me.on_progress(p), &cancel);
            me.update(true, |s| match &r {
                Ok(rep) => {
                    s.job.phase = "done";
                    s.job.result = Some(Done {
                        version: rep.version.clone(),
                        installed: rep.installed,
                        downloaded_bytes: rep.downloaded_bytes,
                        backup: rep.backup.as_ref().map(|p| p.display().to_string()),
                    });
                    s.check = Check { status: "current", installed: Some(rep.version.clone()), server: Some(rep.version.clone()), message: None };
                }
                Err(PatchError::Cancelled) => {
                    s.job.phase = "cancelled";
                    s.job.cancelling = false;
                }
                Err(e) => {
                    let installing = s.job.progress.stage == "install";
                    s.job.phase = "failed";
                    s.job.cancelling = false;
                    s.job.error = Some(friendly(e, installing));
                    // The install may have changed part way: the real version is on disk.
                    s.check.installed = signet_patch::local_version(&me.cfg.install);
                }
            });
            me.busy.store(false, Ordering::SeqCst);
        });
        Ok(self.state())
    }

    pub fn cancel(&self) {
        if self.state.lock().unwrap().job.progress.stage == "install" {
            return;
        }
        self.cancel.lock().unwrap().cancel();
        self.update(true, |s| {
            if matches!(s.job.phase, "planning" | "applying") {
                s.job.cancelling = true;
            }
        });
    }

    /// Back to the idle screen after a plan, result or error.
    pub fn dismiss(&self) -> Value {
        if !self.busy.load(Ordering::SeqCst) {
            self.update(true, |s| {
                s.job = idle_job();
                s.restore = idle_restore();
            });
        }
        self.state()
    }

    /// Put back the files of the newest unrestored backup.
    pub fn restore(self: &Arc<Self>) -> Result<Value, String> {
        let dir = signet_patch::latest_backup(&self.cfg.backups).ok_or("There is no backup to restore.")?;
        if !self.blocked().is_empty() {
            return Err(CLOSE_FIRST.into());
        }
        let _ = self.start_job()?;
        self.update(true, |s| {
            s.job = idle_job();
            s.restore = idle_restore();
            s.restore.phase = "running";
        });
        let me = self.clone();
        std::thread::spawn(move || {
            let r = signet_patch::restore(&dir, Some(&me.cfg.install), &|p| {
                if let Progress::Restoring { done, total, .. } = p {
                    me.update(false, |s| {
                        s.restore.done = done;
                        s.restore.total = total;
                    });
                }
            });
            let installed = signet_patch::local_version(&me.cfg.install);
            me.update(true, |s| {
                s.check.installed = installed.clone();
                match &r {
                    Ok(rep) => {
                        s.restore.phase = "done";
                        s.restore.restored = rep.restored;
                        s.restore.removed = rep.removed;
                        s.restore.partial = rep.was_partial;
                        // Behind the server again, if it is known.
                        if let (Some(i), Some(v)) = (&installed, &s.check.server) {
                            s.check.status = if signet_patch::version_order(i, v).is_lt() { "available" } else { "current" };
                        }
                    }
                    Err(e) => {
                        s.restore.phase = "failed";
                        s.restore.error = Some(friendly(e, false));
                    }
                }
            });
            me.busy.store(false, Ordering::SeqCst);
        });
        Ok(self.state())
    }

    fn on_progress(&self, p: Progress) {
        let quiet = matches!(p, Progress::Downloading { .. } | Progress::Scanning { .. } | Progress::Staged { .. } | Progress::Installing { .. });
        self.update(!quiet, |s| {
            let g = &mut s.job.progress;
            match p {
                Progress::Connecting { .. } | Progress::Version { .. } | Progress::Catalog { .. } => g.stage = "connect",
                Progress::Scanning { done, total } => {
                    g.stage = "scan";
                    g.scanned = done;
                    g.scan_total = total;
                }
                Progress::Planned { bytes, .. } => {
                    g.stage = "download";
                    g.total_bytes = bytes;
                }
                Progress::Downloading { done_bytes, total_bytes } => {
                    g.stage = "download";
                    g.done_bytes = done_bytes;
                    g.total_bytes = total_bytes;
                }
                Progress::Staged { done, total, .. } => {
                    g.verified = done;
                    g.verify_total = total;
                }
                Progress::Installing { done, total, .. } => {
                    g.stage = "install";
                    g.installed = done;
                    g.install_total = total;
                }
                Progress::Restoring { .. } => {}
            }
        });
    }
}

#[cfg(test)]
mod tests;
