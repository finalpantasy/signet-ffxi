//! Retail FINAL FANTASY XI: read-only client checks and the normal PlayOnline launch.
//!
//! Nothing here writes into the install or passes arguments to PlayOnline. Play runs
//! `pol.exe` exactly as its shortcut does; PlayOnline itself handles login and updates.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::winutil::{self, ProcHandle};

/// Where the PlayOnline installer records its folders (value `1000` is the viewer).
const POL_KEY: &str = r"SOFTWARE\WOW6432Node\PlayOnlineUS\InstallFolder";
const POL_VALUE: &str = "1000";
/// The client counts as out of date when its build is this much older than the PlayOnline
/// viewer's last update, or this much older than today (Square Enix patches about monthly).
const BEHIND_VIEWER_DAYS: i64 = 31;
const MAX_AGE_DAYS: i64 = 62;

/// Highest `NNNNNNNN_N` version listed in a `patch.cfg`; the client reports this version at login.
pub fn installed_version(patch_cfg: &str) -> Option<String> {
    patch_cfg
        .split_ascii_whitespace()
        .filter(|t| {
            let b = t.as_bytes();
            b.len() >= 10 && b[..8].iter().all(u8::is_ascii_digit) && b[8] == b'_' && b[9..].iter().all(u8::is_ascii_digit)
        })
        .max_by(|a, b| a[..8].cmp(&b[..8]).then_with(|| a[9..].parse::<u32>().unwrap_or(0).cmp(&b[9..].parse::<u32>().unwrap_or(0))))
        .map(str::to_string)
}

/// Build date of a Square Enix version string: two product digits, then YYMMDD
/// (`30190430_0` is FFXI 2019-04-30, `20261022_0` is PlayOnline 2026-10-22).
pub fn build_date(version: &str) -> Option<(i32, u32, u32)> {
    let d = version.get(2..8)?;
    if !d.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (y, m, day) = (2000 + d[..2].parse::<i32>().ok()?, d[2..4].parse::<u32>().ok()?, d[4..6].parse::<u32>().ok()?);
    ((1..=12).contains(&m) && (1..=31).contains(&day)).then_some((y, m, day))
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days_from_civil((y, m, d): (i32, u32, u32)) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn today_days() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64 / 86_400)
}

/// "outdated", "current" or "unknown" for the installed client.
pub fn client_state(ffxi: Option<&str>, viewer: Option<&str>, today: i64) -> &'static str {
    let Some(built) = ffxi.and_then(build_date).map(days_from_civil) else { return "unknown" };
    let behind_viewer = viewer.and_then(build_date).map(days_from_civil).is_some_and(|v| v - built > BEHIND_VIEWER_DAYS);
    if behind_viewer || today - built > MAX_AGE_DAYS { "outdated" } else { "current" }
}

fn iso((y, m, d): (i32, u32, u32)) -> String {
    format!("{y:04}-{m:02}-{d:02}")
}

#[derive(Default)]
struct Cache {
    dll_key: Option<(u64, u64)>,
    sha256: String,
    cfg_key: Option<(u64, u64)>,
    installed: Option<String>,
}

pub struct Retail {
    pub install: PathBuf,
    pub viewer: PathBuf,
    cache: Mutex<Cache>,
    pol: Mutex<Option<ProcHandle>>,
}

fn file_key(p: &Path) -> Option<(u64, u64)> {
    let m = std::fs::metadata(p).ok()?;
    let t = m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some((m.len(), t))
}

fn read_version(p: &Path) -> Option<String> {
    std::fs::read(p).ok().and_then(|b| installed_version(&String::from_utf8_lossy(&b)))
}

impl Retail {
    pub fn new(install: PathBuf) -> Self {
        let viewer = winutil::reg_string_hklm(POL_KEY, POL_VALUE)
            .map(PathBuf::from)
            .filter(|p| p.join("pol.exe").is_file())
            .or_else(|| install.parent().map(|p| p.join("PlayOnlineViewer")))
            .unwrap_or_default();
        Retail { install, viewer, cache: Mutex::default(), pol: Mutex::default() }
    }

    fn pol_exe(&self) -> PathBuf {
        self.viewer.join("pol.exe")
    }

    /// PID of PlayOnline (FFXI runs inside it), whether Signet started it or not.
    pub fn running(&self) -> Option<u32> {
        let mut g = self.pol.lock().unwrap();
        if let Some(h) = g.as_ref() {
            if h.poll().is_none() {
                return Some(h.pid());
            }
            *g = None;
        }
        winutil::pids_for_exe(&self.pol_exe()).first().copied()
    }

    pub fn info(&self) -> Value {
        let dll = self.install.join("FFXiMain.dll");
        let cfg = self.install.join("patch.cfg");
        let (sha256, installed, dll_meta) = {
            let mut c = self.cache.lock().unwrap();
            let dk = file_key(&dll);
            if dk.is_some() && c.dll_key != dk {
                c.sha256 = std::fs::read(&dll).ok().and_then(|b| winutil::sha256_hex(&b).ok()).unwrap_or_default();
                c.dll_key = dk;
            }
            let ck = file_key(&cfg);
            if c.cfg_key != ck {
                c.installed = read_version(&cfg);
                c.cfg_key = ck;
            }
            (if dk.is_some() { c.sha256.clone() } else { String::new() }, c.installed.clone(), dk)
        };
        let viewer_version = read_version(&self.viewer.join("patch.cfg"));
        let state = client_state(installed.as_deref(), viewer_version.as_deref(), today_days());
        json!({
            "dll": { "path": dll, "found": dll_meta.is_some(), "size": dll_meta.map_or(0, |k| k.0), "modified": dll_meta.map_or(0, |k| k.1), "sha256": sha256 },
            "installed": installed,
            "installedDate": installed.as_deref().and_then(build_date).map(iso),
            "viewer": {
                "path": self.viewer,
                "found": self.pol_exe().is_file(),
                "version": viewer_version,
                "date": viewer_version.as_deref().and_then(build_date).map(iso),
            },
            "state": state,
            "pid": self.running(),
        })
    }

    /// Starts PlayOnline the way its shortcut does. pol.exe asks Windows for approval itself.
    pub fn play(&self) -> Result<u32, String> {
        if let Some(pid) = self.running() {
            return Err(format!("PlayOnline is already running (pid {pid})."));
        }
        let exe = self.pol_exe();
        if !exe.is_file() {
            return Err(format!("PlayOnline was not found at {}.", exe.display()));
        }
        let h = winutil::shell_open(&exe, &self.viewer)?;
        let pid = h.pid();
        *self.pol.lock().unwrap() = Some(h);
        Ok(pid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_cfg_version_is_the_highest_token() {
        let cfg = "file patch.txt {\n30020917_0 833977 6 -1 30020917_0/Direct/patch.txt.slc 3\n30190430_0 2 2 2\n30190426_1 1\n}\nfile x {\n30190305_0 1\n}";
        assert_eq!(installed_version(cfg).as_deref(), Some("30190430_0"));
        assert_eq!(installed_version("nothing here 123_4"), None);
    }

    #[test]
    fn build_dates() {
        assert_eq!(build_date("30190430_0"), Some((2019, 4, 30)));
        assert_eq!(build_date("20261022_0"), Some((2026, 10, 22)));
        assert_eq!(build_date("30191330_0"), None);
        assert_eq!(build_date("3x"), None);
        assert_eq!(days_from_civil((1970, 1, 1)), 0);
        assert_eq!(days_from_civil((2026, 11, 23)) - days_from_civil((2026, 10, 22)), 32);
    }

    #[test]
    fn the_2019_client_is_out_of_date() {
        let today = days_from_civil((2026, 11, 23));
        // This PC: FFXI 2019 build, PlayOnline viewer updated 2026-10-22.
        assert_eq!(client_state(Some("30190430_0"), Some("20261022_0"), today), "outdated");
        assert_eq!(client_state(Some("30261110_0"), Some("20261022_0"), today), "current");
        // Viewer unreadable: age alone decides.
        assert_eq!(client_state(Some("30261006_0"), None, today), "current");
        assert_eq!(client_state(Some("30260801_0"), None, today), "outdated");
        assert_eq!(client_state(None, Some("20261022_0"), today), "unknown");
    }
}
