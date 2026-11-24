//! The 60 Hz parity CSV shared by the retail probe and Signet's `--parity-log`.
//!
//! Coordinates are the game's: x, y (negative is up) and z. `heading_deg` is the character's
//! heading in degrees as the source stores it (retail: the entity's radian heading converted;
//! Signet: the sim yaw). Camera distance, yaw and pitch are derived the same way on both sides
//! from the eye and look-at points, see `derive_camera`.
//!
//! `quality` is a bit set: 0 is a clean sample. 1 = player entity unreadable, 2 = camera
//! unreadable, 4 = a value was not finite or not plausible, 8 = the sample was late (more than
//! 1.5 periods after the previous one). The comparator ignores samples with bits 1, 2 or 4.

use std::fmt::Write as _;
use std::io::Write;
use std::path::Path;

pub const Q_ENTITY: u32 = 1;
pub const Q_CAMERA: u32 = 2;
pub const Q_INSANE: u32 = 4;
pub const Q_LATE: u32 = 8;
pub const Q_BAD_MASK: u32 = Q_ENTITY | Q_CAMERA | Q_INSANE;

pub const HEADER: &str = "t,px,py,pz,heading_deg,speed_eff,speed_srv,eye_x,eye_y,eye_z,look_x,look_y,look_z,dist,cam_yaw_deg,cam_pitch_deg,fov_deg,quality,k_w,k_a,k_s,k_d,k_up,k_down,k_left,k_right,k_lmb,k_rmb,k_mmb,k_num,k_nav,wheel,mouse_dx,mouse_dy";

/// Numpad key bits in `k_num`: bit n is VK_NUMPADn for n in 0..=9, then these.
pub const NUM_ADD: u32 = 1 << 10;
pub const NUM_SUB: u32 = 1 << 11;
pub const NUM_MUL: u32 = 1 << 12;
pub const NUM_DIV: u32 = 1 << 13;
pub const NUM_4: u32 = 1 << 4;
pub const NUM_6: u32 = 1 << 6;
pub const NUM_8: u32 = 1 << 8;
pub const NUM_2: u32 = 1 << 2;
/// Navigation key bits in `k_nav`.
pub const NAV_PGUP: u32 = 1;
pub const NAV_PGDN: u32 = 2;
pub const NAV_HOME: u32 = 4;
pub const NAV_END: u32 = 8;
pub const NAV_INS: u32 = 16;
pub const NAV_DEL: u32 = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Keys {
    pub w: bool,
    pub a: bool,
    pub s: bool,
    pub d: bool,
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    pub lmb: bool,
    pub rmb: bool,
    pub mmb: bool,
    pub num: u32,
    pub nav: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row {
    pub t: f64,
    pub pos: [f64; 3],
    pub heading_deg: f64,
    pub speed_eff: f64,
    pub speed_srv: f64,
    pub eye: [f64; 3],
    pub look: [f64; 3],
    pub dist: f64,
    pub cam_yaw: f64,
    pub cam_pitch: f64,
    pub fov_deg: f64,
    pub quality: u32,
    pub keys: Keys,
    /// Wheel notches since the previous sample (positive is away from the user, zoom in).
    pub wheel: i32,
    /// Raw relative mouse counts since the previous sample.
    pub mouse_dx: i32,
    pub mouse_dy: i32,
}

impl Default for Row {
    fn default() -> Self {
        Row {
            t: 0.0,
            pos: [0.0; 3],
            heading_deg: 0.0,
            speed_eff: f64::NAN,
            speed_srv: f64::NAN,
            eye: [0.0; 3],
            look: [0.0; 3],
            dist: 0.0,
            cam_yaw: 0.0,
            cam_pitch: 0.0,
            fov_deg: f64::NAN,
            quality: 0,
            keys: Keys::default(),
            wheel: 0,
            mouse_dx: 0,
            mouse_dy: 0,
        }
    }
}

/// Distance, yaw and pitch (degrees) of a camera at `eye` looking at `look`, y down.
/// Yaw 0 faces +z and turns toward +x; negative pitch looks down.
pub fn derive_camera(eye: [f64; 3], look: [f64; 3]) -> (f64, f64, f64) {
    let f = [look[0] - eye[0], look[1] - eye[1], look[2] - eye[2]];
    let dist = (f[0] * f[0] + f[1] * f[1] + f[2] * f[2]).sqrt();
    if dist < 1e-9 {
        return (0.0, 0.0, 0.0);
    }
    let yaw = f[0].atan2(f[2]).to_degrees();
    let pitch = (-f[1] / dist).clamp(-1.0, 1.0).asin().to_degrees();
    (dist, yaw, pitch)
}

impl Row {
    /// Fills `dist`, `cam_yaw` and `cam_pitch` from `eye` and `look`.
    pub fn derive(&mut self) {
        let (d, y, p) = derive_camera(self.eye, self.look);
        self.dist = d;
        self.cam_yaw = y;
        self.cam_pitch = p;
    }

    pub fn write_to(&self, out: &mut String) {
        let k = &self.keys;
        let b = |x: bool| x as u8;
        let f = |v: f64| if v.is_finite() { format!("{v:.5}") } else { String::new() };
        let _ = writeln!(
            out,
            "{:.6},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{:#x},{:#x},{},{},{}",
            self.t,
            f(self.pos[0]),
            f(self.pos[1]),
            f(self.pos[2]),
            f(self.heading_deg),
            f(self.speed_eff),
            f(self.speed_srv),
            f(self.eye[0]),
            f(self.eye[1]),
            f(self.eye[2]),
            f(self.look[0]),
            f(self.look[1]),
            f(self.look[2]),
            f(self.dist),
            f(self.cam_yaw),
            f(self.cam_pitch),
            f(self.fov_deg),
            self.quality,
            b(k.w),
            b(k.a),
            b(k.s),
            b(k.d),
            b(k.up),
            b(k.down),
            b(k.left),
            b(k.right),
            b(k.lmb),
            b(k.rmb),
            b(k.mmb),
            k.num,
            k.nav,
            self.wheel,
            self.mouse_dx,
            self.mouse_dy,
        );
    }
}

/// Buffered CSV writer; rows are flushed on `finish` and every 600 rows.
pub struct Writer {
    file: std::fs::File,
    buf: String,
    pub rows: usize,
}

impl Writer {
    pub fn create(path: &Path) -> Result<Writer, String> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
            }
        }
        let mut file = std::fs::File::create(path).map_err(|e| format!("create {}: {e}", path.display()))?;
        writeln!(file, "{HEADER}").map_err(|e| e.to_string())?;
        Ok(Writer { file, buf: String::new(), rows: 0 })
    }
    pub fn push(&mut self, r: &Row) -> Result<(), String> {
        r.write_to(&mut self.buf);
        self.rows += 1;
        if self.rows % 600 == 0 {
            self.flush()?;
        }
        Ok(())
    }
    pub fn flush(&mut self) -> Result<(), String> {
        self.file.write_all(self.buf.as_bytes()).map_err(|e| e.to_string())?;
        self.buf.clear();
        Ok(())
    }
}

fn num(s: &str) -> f64 {
    let s = s.trim();
    if s.is_empty() {
        return f64::NAN;
    }
    if let Some(h) = s.strip_prefix("0x") {
        return u64::from_str_radix(h, 16).map(|v| v as f64).unwrap_or(f64::NAN);
    }
    s.parse().unwrap_or(f64::NAN)
}

/// Parses a parity CSV. Columns are found by name, so extra or reordered columns are fine.
pub fn read(text: &str) -> Result<Vec<Row>, String> {
    let mut lines = text.lines();
    let head = lines.next().ok_or("empty CSV")?;
    let cols: Vec<&str> = head.split(',').map(str::trim).collect();
    let idx = |n: &str| cols.iter().position(|c| *c == n);
    let need = |n: &str| idx(n).ok_or(format!("missing column `{n}`"));
    let (it, ipx, ipy, ipz) = (need("t")?, need("px")?, need("py")?, need("pz")?);
    let ih = need("heading_deg")?;
    let (iex, iey, iez, ilx, ily, ilz) = (need("eye_x")?, need("eye_y")?, need("eye_z")?, need("look_x")?, need("look_y")?, need("look_z")?);
    let (isp_e, isp_s, ifov, iq) = (idx("speed_eff"), idx("speed_srv"), idx("fov_deg"), idx("quality"));
    let (idist, iyaw, ipitch) = (idx("dist"), idx("cam_yaw_deg"), idx("cam_pitch_deg"));
    let key = |n: &str| idx(n);
    let ks = [key("k_w"), key("k_a"), key("k_s"), key("k_d"), key("k_up"), key("k_down"), key("k_left"), key("k_right"), key("k_lmb"), key("k_rmb"), key("k_mmb")];
    let (inum, inav, iwh, idx_, idy) = (key("k_num"), key("k_nav"), key("wheel"), key("mouse_dx"), key("mouse_dy"));
    let mut out = Vec::new();
    for (ln, line) in lines.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        if f.len() < cols.len() {
            return Err(format!("line {}: {} fields, header has {}", ln + 2, f.len(), cols.len()));
        }
        let g = |i: usize| num(f[i]);
        let o = |i: Option<usize>| i.map_or(f64::NAN, |i| num(f[i]));
        let kb = |i: Option<usize>| i.is_some_and(|i| num(f[i]) > 0.5);
        let mut r = Row {
            t: g(it),
            pos: [g(ipx), g(ipy), g(ipz)],
            heading_deg: g(ih),
            speed_eff: o(isp_e),
            speed_srv: o(isp_s),
            eye: [g(iex), g(iey), g(iez)],
            look: [g(ilx), g(ily), g(ilz)],
            fov_deg: o(ifov),
            quality: if iq.is_some() { o(iq) as u32 } else { 0 },
            keys: Keys {
                w: kb(ks[0]),
                a: kb(ks[1]),
                s: kb(ks[2]),
                d: kb(ks[3]),
                up: kb(ks[4]),
                down: kb(ks[5]),
                left: kb(ks[6]),
                right: kb(ks[7]),
                lmb: kb(ks[8]),
                rmb: kb(ks[9]),
                mmb: kb(ks[10]),
                num: o(inum).max(0.0) as u32,
                nav: o(inav).max(0.0) as u32,
            },
            wheel: o(iwh).max(-1e6).min(1e6) as i32,
            mouse_dx: o(idx_).max(-1e6).min(1e6) as i32,
            mouse_dy: o(idy).max(-1e6).min(1e6) as i32,
            ..Default::default()
        };
        r.derive();
        // Trust the file's own derived columns only when present and finite (they match derive()).
        for (v, slot) in [(o(idist), &mut r.dist), (o(iyaw), &mut r.cam_yaw), (o(ipitch), &mut r.cam_pitch)] {
            if v.is_finite() {
                *slot = v;
            }
        }
        out.push(r);
    }
    if out.len() < 2 {
        return Err("fewer than 2 rows".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_matches_signet_camera_convention() {
        // Camera 6 yalms behind and above the target, looking down 15 degrees toward +z.
        let p = (-15.0f64).to_radians();
        let look = [1.0, -1.5, 2.0];
        let f = [0.0, -p.sin(), p.cos()];
        let eye = [look[0] - f[0] * 6.0, look[1] - f[1] * 6.0, look[2] - f[2] * 6.0];
        let (d, y, pitch) = derive_camera(eye, look);
        assert!((d - 6.0).abs() < 1e-9 && y.abs() < 1e-9 && (pitch + 15.0).abs() < 1e-9, "{d} {y} {pitch}");
        // Facing +x is yaw 90.
        let (_, y, _) = derive_camera([0.0; 3], [1.0, 0.0, 0.0]);
        assert!((y - 90.0).abs() < 1e-9);
    }

    #[test]
    fn round_trip() {
        let mut r = Row { t: 1.25, pos: [1.0, -2.0, 3.0], heading_deg: 90.0, eye: [0.0, -3.0, -6.0], look: [0.0, -1.5, 0.0], fov_deg: 57.5, wheel: -2, mouse_dx: 7, quality: 8, ..Default::default() };
        r.keys.w = true;
        r.keys.num = NUM_4 | NUM_ADD;
        r.derive();
        let mut s = String::from(HEADER);
        s.push('\n');
        r.write_to(&mut s);
        r.t = 1.30;
        r.write_to(&mut s);
        let rows = read(&s).unwrap();
        assert_eq!(rows.len(), 2);
        let q = rows[0];
        assert!((q.t - 1.25).abs() < 1e-9 && q.keys.w && !q.keys.a && q.wheel == -2 && q.mouse_dx == 7 && q.quality == 8);
        assert_eq!(q.keys.num, NUM_4 | NUM_ADD);
        assert!(q.speed_eff.is_nan() && (q.fov_deg - 57.5).abs() < 1e-9);
        assert!((q.dist - r.dist).abs() < 1e-4);
    }

    #[test]
    fn missing_column_is_reported() {
        assert!(read("t,px\n1,2\n2,3\n").unwrap_err().contains("missing column"));
    }
}
