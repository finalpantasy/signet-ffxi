//! Update flow against the fake patch server on 127.0.0.1 and a made-up install in a temp folder.
//! Never contacts Square Enix and never touches a real install. Apply and restore refuse while
//! PlayOnline runs on this PC, so close it before running these.

use std::fs;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use signet_patch::fake;

use super::*;

struct Rig {
    up: Arc<Updater>,
    root: PathBuf,
    events: Arc<Mutex<Vec<Value>>>,
}

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("signet-launcher-tests").join(format!("{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn rig(name: &str, current: bool, delay_ms: u64) -> Rig {
    let root = temp(name);
    let built = fake::build(&root.join("fake"), current, 3, 600).unwrap();
    let server = fake::serve(0, Duration::from_millis(delay_ms), &built);
    let cfg = Config { install: built.install.clone(), staging: root.join("staging"), backups: root.join("backups"), server };
    let up = Updater::new(cfg);
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    up.set_emitter(move |v| sink.lock().unwrap().push(v));
    Rig { up, root, events }
}

fn wait(up: &Updater, what: &str, f: impl Fn(&Value) -> bool) -> Value {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        let s = up.state();
        if f(&s) {
            return s;
        }
        assert!(Instant::now() < end, "timed out waiting for {what}: {s}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn phase(s: &Value) -> &str {
    s["job"]["phase"].as_str().unwrap()
}

fn closed_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn offline(r: &Rig) -> Arc<Updater> {
    Updater::new(Config {
        install: r.up.cfg.install.clone(),
        staging: r.root.join("s"),
        backups: r.root.join("b"),
        server: LiveServer { port: closed_port(), ..r.up.cfg.server.clone() },
    })
}

#[test]
fn server_override_is_loopback_only() {
    let get = |v: &'static str| move |k: &str| (k == "SIGNET_PATCH_SERVER").then(|| v.to_string());
    let ok = Config::from_lookup("i".into(), get("127.0.0.1:53999"));
    assert_eq!((ok.server.host.as_str(), ok.server.port), ("127.0.0.1", 53999));
    for bad in ["pc001.pol.com:53001", "10.0.0.5:53001", "127.0.0.1", "127.0.0.1:x", "evil.com:1@127.0.0.1:2"] {
        let c = Config::from_lookup("i".into(), get(bad));
        assert_eq!((c.server.host.as_str(), c.server.port), ("127.0.0.1", 1), "{bad}");
    }
    let real = Config::from_lookup("i".into(), |_| None);
    assert_eq!(real.server.host, "pc001.pol.com");
}

#[test]
fn check_says_up_to_date() {
    let r = rig("check-current", true, 0);
    let s = r.up.check();
    assert_eq!(s["check"]["status"], "current");
    assert_eq!(s["check"]["installed"], fake::OLD);
    assert_eq!(s["check"]["server"], fake::OLD);
}

#[test]
fn check_says_update_available_and_writes_nothing() {
    let r = rig("check-available", false, 0);
    let s = r.up.check();
    assert_eq!(s["check"]["status"], "available");
    assert_eq!((s["check"]["installed"].as_str(), s["check"]["server"].as_str()), (Some(fake::OLD), Some(fake::NEW)));
    assert!(!r.root.join("staging").exists() && !r.root.join("backups").exists());
}

#[test]
fn check_unreachable_is_a_status_not_an_error() {
    let r = rig("check-offline", false, 0);
    let s = offline(&r).check();
    assert_eq!(s["check"]["status"], "offline");
    assert_eq!(s["check"]["message"], UNREACHABLE);
    assert_eq!(s["check"]["installed"], fake::OLD);
}

#[test]
fn plan_apply_progress_and_restore() {
    let r = rig("flow", false, 0);
    let install = r.up.cfg.install.clone();
    let before = fs::read(install.join("ROM/1/0.DAT")).unwrap();
    r.up.check();

    r.up.plan().unwrap();
    let s = wait(&r.up, "plan", |s| phase(s) == "planned");
    let plan = &s["job"]["plan"];
    assert_eq!(plan["version"], fake::NEW);
    assert!(plan["files"].as_u64().unwrap() >= 4 && plan["bytes"].as_u64().unwrap() > 1_000_000, "{plan}");
    assert_eq!(fs::read(install.join("ROM/1/0.DAT")).unwrap(), before, "plan must not write");

    r.up.apply().unwrap();
    let s = wait(&r.up, "apply", |s| phase(s) == "done");
    let done = &s["job"]["result"];
    assert_eq!(done["version"], fake::NEW);
    assert!(done["installed"].as_u64().unwrap() >= 5);
    assert!(PathBuf::from(done["backup"].as_str().unwrap()).join("signet-backup.txt").is_file());
    assert_ne!(fs::read(install.join("ROM/1/0.DAT")).unwrap(), before);
    assert_eq!(s["check"]["status"], "current");
    assert_eq!(signet_patch::local_version(&install).as_deref(), Some(fake::NEW));

    // Progress came through in order: download bytes only grow, install comes last.
    let ev = r.events.lock().unwrap();
    let stages: Vec<&str> = ev.iter().filter_map(|e| e["job"]["progress"]["stage"].as_str()).collect();
    let first = |n: &str| stages.iter().position(|s| *s == n);
    assert!(first("scan") < first("download") && first("download") < first("install"), "{stages:?}");
    let bytes: Vec<u64> = ev.iter().filter_map(|e| e["job"]["progress"]["done_bytes"].as_u64()).collect();
    assert!(bytes.windows(2).all(|w| w[0] <= w[1]) && *bytes.last().unwrap() > 1_000_000);
    drop(ev);

    // Restore the newest backup: files and version come back, and the backup is spent.
    assert!(s["backup"]["path"].is_string());
    r.up.restore().unwrap();
    let s = wait(&r.up, "restore", |s| s["restore"]["phase"] == "done");
    assert!(s["restore"]["restored"].as_u64().unwrap() >= 5);
    assert_eq!(fs::read(install.join("ROM/1/0.DAT")).unwrap(), before);
    assert_eq!(signet_patch::local_version(&install).as_deref(), Some(fake::OLD));
    assert!(s["backup"].is_null());
    assert_eq!(s["check"]["status"], "available");
    assert!(r.up.restore().is_err());
}

#[test]
fn cancel_during_download_changes_nothing() {
    let r = rig("cancel", false, 60);
    let install = r.up.cfg.install.clone();
    let cfg_before = fs::read(install.join("patch.cfg")).unwrap();
    r.up.apply().unwrap();
    wait(&r.up, "download to start", |s| s["job"]["progress"]["stage"] == "download" && s["job"]["progress"]["done_bytes"].as_u64().unwrap_or(0) > 0);
    r.up.cancel();
    let s = wait(&r.up, "cancel", |s| phase(s) == "cancelled");
    assert_eq!(s["job"]["cancelling"], false);
    assert_eq!(fs::read(install.join("patch.cfg")).unwrap(), cfg_before);
    assert!(s["backup"].is_null());
    r.up.dismiss();
    assert_eq!(phase(&r.up.state()), "idle");
}

#[test]
fn plan_failure_is_plain_text() {
    let r = rig("plan-offline", false, 0);
    let up = offline(&r);
    up.plan().unwrap();
    let s = wait(&up, "plan failure", |s| phase(s) == "failed");
    assert_eq!(s["job"]["error"], UNREACHABLE);
}

#[test]
fn friendly_messages() {
    assert_eq!(friendly(&PatchError::refused("close the game first: pol.exe (pid 4)"), false), CLOSE_FIRST);
    assert_eq!(friendly(&PatchError::failed("cannot connect to pc001.pol.com:53001: timed out"), false), UNREACHABLE);
    assert!(friendly(&PatchError::failed("disk full"), true).contains("Restore previous files"));
}
