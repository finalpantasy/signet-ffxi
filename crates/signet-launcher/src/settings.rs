//! Launcher settings: one JSON file under `%APPDATA%\Signet\launcher.json`.
//! Every value that reaches a command line is validated here first.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Default FFXI install when `SIGNET_INSTALL` is not set (read only; the launcher never writes into it).
pub const DEFAULT_INSTALL: &str = r"C:\Program Files (x86)\PlayOnline\SquareEnix\FINAL FANTASY XI";

/// Resolutions offered for the Signet window (`signet-render --size`).
pub const RESOLUTIONS: &[(u32, u32)] = &[(1280, 720), (1600, 900), (1920, 1080), (2560, 1440), (3840, 2160)];
/// signet-render's engineering bounds for `--speed-mod` (play.rs MIN/MAX_SPEED_PCT).
pub const SPEED_MIN: i32 = -50;
pub const SPEED_MAX: i32 = 100;
const RECENT_MAX: usize = 8;
const FAVORITES_MAX: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CameraMode {
    Retail,
    Free,
}

impl CameraMode {
    pub fn flag(self) -> &'static str {
        match self {
            CameraMode::Retail => "retail",
            CameraMode::Free => "free",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub zone_id: u16,
    pub camera_mode: CameraMode,
    pub speed_mod: i32,
    pub theme: Theme,
    pub width: u32,
    pub height: u32,
    pub favorites: Vec<u16>,
    pub recent: Vec<u16>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            version: 1,
            zone_id: 100,
            camera_mode: CameraMode::Retail,
            speed_mod: 0,
            theme: Theme::System,
            width: 1920,
            height: 1080,
            favorites: Vec::new(),
            recent: Vec::new(),
        }
    }
}

/// A partial update sent by the UI. Unknown fields are rejected.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    pub zone_id: Option<u16>,
    pub camera_mode: Option<CameraMode>,
    pub speed_mod: Option<i32>,
    pub theme: Option<Theme>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub favorite: Option<(u16, bool)>,
}

/// Rounds like signet-render (even percent, ties away from zero) and clamps.
pub fn even_speed(pct: i32) -> i32 {
    let half = pct as f32 / 2.0;
    ((half.round() as i32) * 2).clamp(SPEED_MIN, SPEED_MAX)
}

impl Settings {
    /// Applies a patch. `zone_ok` says whether a zone id exists in the install.
    pub fn apply(&mut self, p: Patch, zone_ok: &dyn Fn(u16) -> bool) -> Result<(), String> {
        let mut next = self.clone();
        if let Some(z) = p.zone_id {
            if !zone_ok(z) {
                return Err(format!("zone {z} is not in this install"));
            }
            next.zone_id = z;
        }
        if let Some(c) = p.camera_mode {
            next.camera_mode = c;
        }
        if let Some(s) = p.speed_mod {
            if !(SPEED_MIN..=SPEED_MAX).contains(&s) {
                return Err(format!("speed-mod must be {SPEED_MIN}..{SPEED_MAX}"));
            }
            next.speed_mod = even_speed(s);
        }
        if let Some(t) = p.theme {
            next.theme = t;
        }
        match (p.width, p.height) {
            (None, None) => {}
            (Some(w), Some(h)) if RESOLUTIONS.contains(&(w, h)) => {
                next.width = w;
                next.height = h;
            }
            _ => return Err("unsupported resolution".into()),
        }
        if let Some((z, on)) = p.favorite {
            if !zone_ok(z) {
                return Err(format!("zone {z} is not in this install"));
            }
            next.favorites.retain(|f| *f != z);
            if on {
                next.favorites.insert(0, z);
                next.favorites.truncate(FAVORITES_MAX);
            }
        }
        *self = next;
        Ok(())
    }

    pub fn push_recent(&mut self, zone: u16) {
        self.recent.retain(|z| *z != zone);
        self.recent.insert(0, zone);
        self.recent.truncate(RECENT_MAX);
    }

    /// Repairs a loaded file so nothing invalid survives a hand edit.
    pub fn sanitize(&mut self) {
        self.version = 1;
        self.speed_mod = even_speed(self.speed_mod);
        if !RESOLUTIONS.contains(&(self.width, self.height)) {
            self.width = 1920;
            self.height = 1080;
        }
        let mut seen = std::collections::HashSet::new();
        self.favorites.retain(|z| seen.insert(*z));
        self.favorites.truncate(FAVORITES_MAX);
        let mut seen = std::collections::HashSet::new();
        self.recent.retain(|z| seen.insert(*z));
        self.recent.truncate(RECENT_MAX);
    }
}

pub fn settings_dir() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Signet")
}

pub fn settings_path() -> PathBuf {
    settings_dir().join("launcher.json")
}

pub fn load(path: &Path) -> Settings {
    let mut s = std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<Settings>(&b).ok()).unwrap_or_default();
    s.sanitize();
    s
}

/// Writes through a temp file and a rename, so a crash never leaves half a file.
pub fn save(path: &Path, s: &Settings) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(s).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, body).map_err(|e| format!("write settings: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("save settings: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn any(_: u16) -> bool {
        true
    }

    #[test]
    fn speed_rounds_like_signet_render() {
        assert_eq!(even_speed(12), 12);
        assert_eq!(even_speed(13), 14);
        assert_eq!(even_speed(-13), -14);
        assert_eq!(even_speed(250), 100);
        assert_eq!(even_speed(-90), -50);
    }

    #[test]
    fn patch_rejects_bad_values_without_partial_writes() {
        let mut s = Settings::default();
        let bad = Patch { camera_mode: Some(CameraMode::Free), width: Some(1234), height: Some(5), ..Default::default() };
        assert!(s.apply(bad, &any).is_err());
        assert_eq!(s.camera_mode, CameraMode::Retail, "a rejected patch changes nothing");
        assert!(s.apply(Patch { speed_mod: Some(101), ..Default::default() }, &any).is_err());
        assert!(s.apply(Patch { width: Some(1920), ..Default::default() }, &any).is_err());
        assert!(s.apply(Patch { zone_id: Some(9), ..Default::default() }, &|z| z != 9).is_err());
        s.apply(Patch { width: Some(2560), height: Some(1440), speed_mod: Some(25), ..Default::default() }, &any).unwrap();
        assert_eq!((s.width, s.height, s.speed_mod), (2560, 1440, 26));
    }

    #[test]
    fn patch_json_rejects_unknown_fields() {
        assert!(serde_json::from_str::<Patch>(r#"{"exe":"calc.exe"}"#).is_err());
        assert!(serde_json::from_str::<Patch>(r#"{"camera_mode":"orbit"}"#).is_err());
        let p: Patch = serde_json::from_str(r#"{"favorite":[230,true]}"#).unwrap();
        assert_eq!(p.favorite, Some((230, true)));
    }

    #[test]
    fn favorites_and_recent_stay_unique_and_bounded() {
        let mut s = Settings::default();
        for z in [1, 2, 1, 3] {
            s.apply(Patch { favorite: Some((z, true)), ..Default::default() }, &any).unwrap();
        }
        assert_eq!(s.favorites, vec![3, 1, 2]);
        s.apply(Patch { favorite: Some((1, false)), ..Default::default() }, &any).unwrap();
        assert_eq!(s.favorites, vec![3, 2]);
        for z in 0..20 {
            s.push_recent(z);
        }
        s.push_recent(15);
        assert_eq!(s.recent.len(), RECENT_MAX);
        assert_eq!(s.recent[0], 15);
    }

    #[test]
    fn load_repairs_hand_edits_and_roundtrips() {
        let dir = std::env::temp_dir().join(format!("signet-launcher-test-{}", std::process::id()));
        let path = dir.join("launcher.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, r#"{"speed_mod":999,"width":7,"height":7,"recent":[5,5,6]}"#).unwrap();
        let s = load(&path);
        assert_eq!((s.speed_mod, s.width, s.recent.clone()), (100, 1920, vec![5, 6]));
        save(&path, &s).unwrap();
        assert_eq!(load(&path), s);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
