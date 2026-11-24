//! Explore zones: launches `signet-render --play` for the chosen zone (offline, no server)
//! and tracks the one process it started.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::settings::Settings;

/// CreateProcess flag: below-normal priority, like every heavy process on this PC.
pub const BELOW_NORMAL: u32 = 0x0000_4000;
/// CreateProcess flag: a console program gets no console window.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Spawn spot for a zone: x, z and facing yaw (ground height is found by the renderer).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spawn {
    pub x: f32,
    pub z: f32,
    pub yaw: f32,
}

/// Looks for signet-render.exe next to the launcher, then in the workspace target folders
/// (release, then debug), also from a side target dir such as target/launcher-v2/release.
pub fn find_exe() -> Option<PathBuf> {
    let me = std::env::current_exe().ok()?;
    let dir = me.parent()?;
    let mut candidates = vec![dir.join("signet-render.exe")];
    for up in dir.ancestors().skip(1).take(3) {
        candidates.push(up.join("release").join("signet-render.exe"));
        candidates.push(up.join("debug").join("signet-render.exe"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// True when the renderer build parses `--zone` (the flag is being added in parallel).
pub fn supports_zone_flag(exe: &Path) -> bool {
    // The option name is a string literal in the binary once the parser knows it.
    std::fs::read(exe).is_ok_and(|b| b.windows(6).any(|w| w == b"--zone"))
}

/// The argument list. Every value comes from validated settings or the install's zone table.
pub fn args(s: &Settings, zone_id: u16, zone_dat: &Path, root: &Path, spawn: Option<Spawn>, zone_flag: bool) -> Vec<String> {
    let mut a = Vec::new();
    if zone_flag {
        a.push(format!("--zone={zone_id}"));
    } else {
        a.push(zone_dat.display().to_string());
    }
    a.push(format!("--root={}", root.display()));
    a.push("--play".into());
    a.push(format!("--camera-mode={}", s.camera_mode.flag()));
    a.push(format!("--speed-mod={}", crate::settings::even_speed(s.speed_mod)));
    a.push(format!("--size={}x{}", s.width, s.height));
    if let Some(sp) = spawn {
        a.push(format!("--actor={:.1},auto,{:.1},{:.0}", sp.x, sp.z, sp.yaw));
    }
    a
}

#[derive(Default)]
pub struct Game {
    child: Mutex<Option<(Child, u16)>>,
    last_exit: Mutex<Option<(i32, String)>>,
}

impl Game {
    /// (pid, zone) while the launched renderer is alive.
    pub fn running(&self) -> Option<(u32, u16)> {
        let mut g = self.child.lock().unwrap();
        if let Some((c, z)) = g.as_mut() {
            match c.try_wait() {
                Ok(None) => return Some((c.id(), *z)),
                Ok(Some(st)) => {
                    let code = st.code().unwrap_or(-1);
                    let tail = std::fs::read_to_string(log_path()).ok().and_then(|t| t.lines().rev().find(|l| !l.trim().is_empty()).map(str::to_string)).unwrap_or_default();
                    *self.last_exit.lock().unwrap() = Some((code, tail));
                    *g = None;
                }
                Err(_) => *g = None,
            }
        }
        None
    }

    pub fn last_exit(&self) -> Option<(i32, String)> {
        self.last_exit.lock().unwrap().clone()
    }

    pub fn launch(&self, exe: &Path, args: &[String], zone: u16) -> Result<u32, String> {
        if let Some((pid, _)) = self.running() {
            return Err(format!("The zone viewer is already open (pid {pid})."));
        }
        let log = log_path();
        std::fs::create_dir_all(log.parent().unwrap()).map_err(|e| e.to_string())?;
        let out = std::fs::File::create(&log).map_err(|e| format!("open log: {e}"))?;
        let err = out.try_clone().map_err(|e| e.to_string())?;
        let child = Command::new(exe)
            .args(args)
            .current_dir(exe.parent().unwrap_or(Path::new(".")))
            .stdin(Stdio::null())
            .stdout(Stdio::from(out))
            .stderr(Stdio::from(err))
            // signet-render is a console program: no console window, output goes to the log file.
            .creation_flags(BELOW_NORMAL | CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("start signet-render: {e}"))?;
        let pid = child.id();
        *self.last_exit.lock().unwrap() = None;
        *self.child.lock().unwrap() = Some((child, zone));
        Ok(pid)
    }

    /// Closes the viewer the way its close button does (WM_CLOSE); if it is still alive after
    /// `grace`, ends that one process (it is our own child, found by handle, never by name).
    pub fn stop(&self, grace: Duration) -> Result<(), String> {
        let Some((pid, _)) = self.running() else { return Ok(()) };
        // The window appears only after the zone loads (several seconds), so keep asking until
        // one has been told to close.
        let mut asked = crate::winutil::close_windows_of(pid) > 0;
        let until = Instant::now() + grace;
        while Instant::now() < until {
            if self.running().is_none() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
            if !asked {
                asked = crate::winutil::close_windows_of(pid) > 0;
            }
        }
        let mut g = self.child.lock().unwrap();
        if let Some((c, _)) = g.as_mut() {
            if c.try_wait().ok().flatten().is_none() {
                c.kill().map_err(|e| format!("could not end signet-render (pid {pid}): {e}"))?;
                let _ = c.wait();
            }
        }
        drop(g);
        self.running();
        Ok(())
    }
}

pub fn log_path() -> PathBuf {
    crate::settings::settings_dir().join("logs").join("signet-render.log")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::CameraMode;

    #[test]
    fn args_use_equals_form_for_negative_values() {
        let mut s = Settings::default();
        s.speed_mod = -13;
        s.camera_mode = CameraMode::Free;
        let a = args(&s, 100, Path::new(r"E:\ffxi\ROM\0\120.DAT"), Path::new(r"E:\ffxi"), Some(Spawn { x: -92.0, z: 77.0, yaw: 270.0 }), false);
        assert_eq!(a[0], r"E:\ffxi\ROM\0\120.DAT");
        assert!(a.contains(&"--play".to_string()));
        assert!(a.contains(&"--speed-mod=-14".to_string()));
        assert!(a.contains(&"--camera-mode=free".to_string()));
        assert!(a.contains(&"--size=1920x1080".to_string()));
        assert!(a.contains(&"--actor=-92.0,auto,77.0,270".to_string()));
        let z = args(&s, 230, Path::new("x.DAT"), Path::new("r"), None, true);
        assert_eq!(z[0], "--zone=230");
        assert!(!z.iter().any(|x| x.starts_with("--actor")));
    }
}
