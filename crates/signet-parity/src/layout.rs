//! Signatures for the retail FFXiMain.dll image and the resolved layout they produce.
//!
//! Every value is found by byte pattern, never by fixed address. Operands (addresses, field
//! offsets) are wildcards that the resolver captures, so a patched build resolves as long as the
//! shape of the code is unchanged. Each target has a primary signature and, where the code has a
//! second independent site, a cross-check that must agree with it.
//!
//! What each signature anchors (2019 build, RVAs from the unpacked image):
//! * `camera_getter`: tiny accessor `mov eax,[G]; mov eax,[eax+0x50]; ret` next to an
//!   `add eax,0x114` accessor. G is the game-state global, the camera object is `[G]+0x50`.
//! * `camera_layout`: the camera's distance function (`sqrt` of look-at minus eye); its operands
//!   are the look-at and eye offsets in the camera object.
//! * `entity_table`: the player-entity accessor `movsx eax,[selector]; mov eax,[table+eax*4]`,
//!   copied at several call sites that must all agree.
//! * `entity_speed`: the entity constructor's stores of the default speed (5.0) into the
//!   server-speed, effective-speed and aux-speed fields.
//! * `entity_pose`: the entity constructor's zeroing of position, rotation, id and name fields.
//!   Cross-checked by `entity_yaw_alt` and `entity_pos_alt`, which are the same fields as written
//!   by the 0x00D character-update handler.
//! * `projection`: the projection setup, `2*atan(192/350)` through `fpatan` and the D3D8
//!   `SetTransform(PROJECTION)` call through vtable slot 0x94.
//! * `dist_min`, `dist_max`: the camera's soft distance limits, 3.0 and 6.0.

use crate::pe::PeInfo;
use crate::sig::{self, Cap, Mode, Sig, Width};

macro_rules! cap {
    ($n:expr, $a:expr, $w:ident) => {
        Cap { name: $n, at: $a, width: Width::$w }
    };
}

static CAMERA_GETTER: Sig = Sig {
    name: "camera_getter",
    pattern: "a1 ?? ?? ?? ?? 8b 40 ?? c3 ?? ?? ?? ?? ?? ?? ?? a1 ?? ?? ?? ?? 05 ?? ?? 00 00 c3",
    caps: &[cap!("global", 1, U32), cap!("ptr_off", 7, U8), cap!("global2", 17, U32)],
    mode: Mode::Unique,
};
static CAMERA_LAYOUT: Sig = Sig {
    name: "camera_layout",
    pattern: "d9 41 ?? d8 61 ?? d9 41 ?? d8 61 ?? d9 41 ?? d8 61 ?? d9 c2 de cb",
    caps: &[cap!("look0", 2, U8), cap!("eye0", 5, U8), cap!("look1", 8, U8), cap!("eye1", 11, U8), cap!("look2", 14, U8), cap!("eye2", 17, U8)],
    mode: Mode::Unique,
};
static ENTITY_TABLE: Sig = Sig {
    name: "entity_table",
    pattern: "0f bf 05 ?? ?? ?? ?? 8b 04 85 ?? ?? ?? ?? 85 c0",
    caps: &[cap!("selector", 3, U32), cap!("table", 10, U32)],
    mode: Mode::Consensus,
};
static ENTITY_SPEED: Sig = Sig {
    name: "entity_speed",
    pattern: "89 86 ?? ?? 00 00 89 86 ?? ?? 00 00 89 86 ?? ?? 00 00 66 89 9e ?? ?? 00 00",
    caps: &[cap!("server", 2, U32), cap!("effective", 8, U32), cap!("aux", 14, U32)],
    mode: Mode::Unique,
};
static ENTITY_POSE: Sig = Sig {
    name: "entity_pose",
    pattern: "89 5e ?? 89 5e ?? 89 5e ?? 89 5e ?? 89 5e ?? 89 5e ?? 89 5e ?? 89 5e ?? 89 5e ?? 89 5e ?? 89 5e ?? 88 5e ?? 89 9e ?? ?? 00 00 f3 ab",
    caps: &[cap!("pos_z", 11, U8), cap!("pos_y", 14, U8), cap!("pos_x", 17, U8), cap!("rot_z", 20, U8), cap!("yaw", 23, U8), cap!("rot_x", 26, U8), cap!("id", 32, U8), cap!("name", 35, U8)],
    mode: Mode::Unique,
};
static ENTITY_YAW_ALT: Sig = Sig {
    name: "entity_yaw_alt",
    pattern: "8b 04 8d ?? ?? ?? ?? c7 40 ?? 00 00 00 00 d9 58 ?? 83 c0 ?? c7 40 ?? 00 00 00 00",
    caps: &[cap!("table", 3, U32), cap!("yaw", 16, U8)],
    mode: Mode::Unique,
};
static ENTITY_POS_ALT: Sig = Sig {
    name: "entity_pos_alt",
    pattern: "d9 58 ?? 83 c0 ?? d9 58 ?? 89 48 ??",
    caps: &[cap!("x", 2, U8), cap!("add", 5, U8), cap!("y_rel", 8, U8), cap!("z_rel", 11, U8)],
    mode: Mode::Consensus,
};
static PROJECTION: Sig = Sig {
    name: "projection",
    pattern: "dd 05 ?? ?? ?? ?? dd 05 ?? ?? ?? ?? d9 f3 8b ?? dc c0 d9 1c 24 53 e8 ?? ?? ?? ?? 50 6a 03 56 ff ?? ?? 00 00 00",
    caps: &[cap!("num", 2, U32), cap!("den", 8, U32), cap!("slot", 33, U32)],
    mode: Mode::Unique,
};
/// Post-2019 shape: the FOV denominator is a float field of the game-state object (`[G]+off`)
/// instead of a constant double. `g` must equal the camera getter's global.
static PROJECTION_FIELD: Sig = Sig {
    name: "projection_field",
    pattern: "a1 ?? ?? ?? ?? d9 81 ?? ?? ?? ?? d9 80 ?? ?? 00 00 d8 c1 d9 5c 24 0c d9 80 ?? ?? 00 00 d8 c1 d9 5c 24 08 dd d8 d9 80 ?? ?? 00 00 d8 88 ?? ?? 00 00 d8 3d ?? ?? ?? ?? d9 5c 24 04 dd 05 ?? ?? ?? ?? d9 80 ?? ?? 00 00 d9 f3 dc c0 d9 1c 24 52 e8 ?? ?? ?? ?? a1 ?? ?? ?? ?? 8d 54 24 18 52 6a 03 8b 80 ?? ?? 00 00 50 8b 08 ff 91 ?? 00 00 00",
    caps: &[cap!("g", 1, U32), cap!("num", 61, U32), cap!("off", 67, U32), cap!("slot", 107, U32)],
    mode: Mode::Unique,
};
static DIST_MIN: Sig = Sig {
    name: "dist_min",
    pattern: "d9 44 24 10 d8 1d ?? ?? ?? ?? df e0 f6 c4 05 7a ?? d9 05 ?? ?? ?? ?? d8 64 24 10",
    caps: &[cap!("cmp", 6, U32), cap!("sub", 19, U32)],
    mode: Mode::Unique,
};
static DIST_MAX: Sig = Sig {
    name: "dist_max",
    pattern: "d9 44 24 10 d8 1d ?? ?? ?? ?? df e0 25 00 01 00 00 75 ?? d9 44 24 10 d8 25 ?? ?? ?? ?? 51 d8 0d",
    caps: &[cap!("cmp", 6, U32), cap!("sub", 25, U32)],
    mode: Mode::Unique,
};

/// Everything the probe needs to read the live process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// RVA of the game-state global; the camera object pointer is at `[[base+rva]+cam_ptr_off]`.
    pub cam_global_rva: u32,
    pub cam_ptr_off: u32,
    pub cam_look_off: u32,
    pub cam_eye_off: u32,
    /// RVA of the entity pointer table (1024+ dwords) and of the player's index word.
    pub ent_table_rva: u32,
    pub ent_selector_rva: u32,
    pub ent_speed_server_off: u32,
    pub ent_speed_eff_off: u32,
    pub ent_speed_aux_off: u32,
    pub ent_pos_off: u32,
    pub ent_yaw_off: u32,
    /// Server id and name offsets, present when the constructor signature resolved.
    pub ent_id_off: Option<u32>,
    pub ent_name_off: Option<u32>,
    /// RVAs of the two doubles whose ratio is the projection FOV: `2*atan(num/den)`.
    pub fov_num_rva: u32,
    pub fov_den: FovDen,
    pub set_transform_slot: u32,
    /// RVAs of the floats the camera compares its distance with.
    pub dist_min_rva: u32,
    pub dist_max_rva: u32,
}

/// Where the FOV denominator lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FovDen {
    /// A constant double at this RVA (2019 build).
    Const(u32),
    /// A float32 field: `[[base+global_rva]+off]` (runtime adjustable FOV, 30261102_1).
    Field { global_rva: u32, off: u32 },
}

impl FovDen {
    pub fn describe(&self) -> String {
        match self {
            FovDen::Const(r) => format!("constant double at RVA {r:#x}"),
            FovDen::Field { global_rva, off } => format!("float field [[RVA {global_rva:#x}]+{off:#x}]"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct CheckLine {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

pub struct Resolution {
    pub layout: Option<Layout>,
    pub lines: Vec<CheckLine>,
}

impl Resolution {
    pub fn ok(&self) -> bool {
        self.layout.is_some() && self.lines.iter().all(|l| l.ok)
    }
}

/// Reads the `f32` at `rva` in a mapped image.
pub fn f32_at(image: &[u8], rva: u32) -> Option<f32> {
    Some(f32::from_le_bytes(image.get(rva as usize..rva as usize + 4)?.try_into().ok()?))
}
pub fn f64_at(image: &[u8], rva: u32) -> Option<f64> {
    Some(f64::from_le_bytes(image.get(rva as usize..rva as usize + 8)?.try_into().ok()?))
}

/// FOV in degrees from the projection constants.
/// Reads the denominator from a mapped image where it is a constant (static check only).
pub fn fov_den_static(image: &[u8], d: FovDen) -> Option<f64> {
    match d {
        FovDen::Const(r) => f64_at(image, r),
        FovDen::Field { .. } => None,
    }
}

pub fn fov_deg(num: f64, den: f64) -> f64 {
    2.0 * (num / den).atan().to_degrees()
}

/// Resolves every signature on a mapped image. `base` is where the image sits in memory (live) or
/// its preferred base (static); absolute operands are converted to RVAs against it, falling back
/// to the preferred base for an unrelocated operand.
pub fn resolve_layout(image: &[u8], pe: &PeInfo, base: u32) -> Resolution {
    let mut lines: Vec<CheckLine> = Vec::new();
    let ranges = pe.code_ranges();
    let size = pe.size_of_image;
    let to_rva = |abs: u32| -> Option<u32> {
        let r = abs.wrapping_sub(base);
        if r < size {
            return Some(r);
        }
        let r = abs.wrapping_sub(pe.image_base);
        (r < size).then_some(r)
    };
    let ok_line = |lines: &mut Vec<CheckLine>, name: &str, ok: bool, detail: String| lines.push(CheckLine { name: name.into(), ok, detail });

    let run = |s: &Sig, lines: &mut Vec<CheckLine>| -> Option<sig::Hit> {
        match sig::resolve(s, image, &ranges) {
            Ok(h) => {
                let at = h.at.iter().take(3).map(|a| format!("{a:#x}")).collect::<Vec<_>>().join(",");
                let more = if h.at.len() > 3 { format!(" (+{} more)", h.at.len() - 3) } else { String::new() };
                lines.push(CheckLine { name: format!("sig {}", s.name), ok: true, detail: format!("{} site(s) at RVA {at}{more}", h.at.len()) });
                Some(h)
            }
            Err(e) => {
                lines.push(CheckLine { name: format!("sig {}", s.name), ok: false, detail: e });
                None
            }
        }
    };

    // Camera.
    let cg = run(&CAMERA_GETTER, &mut lines);
    let cl = run(&CAMERA_LAYOUT, &mut lines);
    let (mut cam_global_rva, mut cam_ptr_off) = (None, None);
    if let Some(h) = &cg {
        let (g, g2) = (h.get("global").unwrap(), h.get("global2").unwrap());
        match (to_rva(g), g == g2) {
            (Some(r), true) => {
                cam_global_rva = Some(r);
                cam_ptr_off = h.get("ptr_off");
                ok_line(&mut lines, "camera global", true, format!("game-state global RVA {r:#x}, camera pointer at +{:#x}", h.get("ptr_off").unwrap()));
            }
            _ => ok_line(&mut lines, "camera global", false, format!("global {g:#x} vs {g2:#x}, or outside the image")),
        }
    }
    let (mut cam_look_off, mut cam_eye_off) = (None, None);
    if let Some(h) = &cl {
        let v = |n: &str| h.get(n).unwrap();
        let good = v("look1") == v("look0") + 4 && v("look2") == v("look0") + 8 && v("eye1") == v("eye0") + 4 && v("eye2") == v("eye0") + 8;
        if good {
            cam_look_off = Some(v("look0"));
            cam_eye_off = Some(v("eye0"));
            ok_line(&mut lines, "camera layout", true, format!("look-at +{:#x}, eye +{:#x} (three consecutive floats each)", v("look0"), v("eye0")));
        } else {
            ok_line(&mut lines, "camera layout", false, "operands are not consecutive xyz triples".into());
        }
    }

    // Entity.
    let et = run(&ENTITY_TABLE, &mut lines);
    let es = run(&ENTITY_SPEED, &mut lines);
    let ep = run(&ENTITY_POSE, &mut lines);
    let ey = run(&ENTITY_YAW_ALT, &mut lines);
    let epa = run(&ENTITY_POS_ALT, &mut lines);
    let (mut table_rva, mut sel_rva) = (None, None);
    if let Some(h) = &et {
        match (to_rva(h.get("table").unwrap()), to_rva(h.get("selector").unwrap())) {
            (Some(t), Some(s)) => {
                table_rva = Some(t);
                sel_rva = Some(s);
                ok_line(&mut lines, "entity table", true, format!("table RVA {t:#x}, player-index word RVA {s:#x}"));
            }
            _ => ok_line(&mut lines, "entity table", false, "operands outside the image".into()),
        }
    }
    if let (Some(h), Some(t)) = (&ey, table_rva) {
        let same = to_rva(h.get("table").unwrap()) == Some(t);
        ok_line(&mut lines, "entity table cross-check", same, "the 0x00D handler indexes the same table".into());
    }
    let speeds = es.as_ref().map(|h| (h.get("server").unwrap(), h.get("effective").unwrap(), h.get("aux").unwrap()));
    if let Some((s, e, a)) = speeds {
        let sane = s < 0x400 && e < 0x400 && a < 0x400 && s != e;
        ok_line(&mut lines, "entity speed", sane, format!("server speed +{s:#x}, effective speed +{e:#x}, aux +{a:#x} (floats)"));
    }
    let mut pos_off = None;
    let mut yaw_off = None;
    let (mut id_off, mut name_off) = (None, None);
    if let Some(h) = &ep {
        let v = |n: &str| h.get(n).unwrap();
        let good = v("pos_y") == v("pos_x") + 4 && v("pos_z") == v("pos_x") + 8 && v("rot_x") + 4 == v("yaw") && v("rot_z") == v("yaw") + 4;
        if good {
            pos_off = Some(v("pos_x"));
            yaw_off = Some(v("yaw"));
            id_off = Some(v("id"));
            name_off = Some(v("name"));
            ok_line(&mut lines, "entity pose", true, format!("position +{:#x} (x,y,z floats), heading +{:#x}, id +{:#x}, name +{:#x}", v("pos_x"), v("yaw"), v("id"), v("name")));
        } else {
            ok_line(&mut lines, "entity pose", false, "position or rotation offsets are not consecutive".into());
        }
    }
    if let Some(h) = &ey {
        let y = h.get("yaw").unwrap();
        match yaw_off {
            Some(p) => ok_line(&mut lines, "heading cross-check", p == y, format!("0x00D handler writes heading at +{y:#x}, constructor says +{p:#x}")),
            None => {
                yaw_off = Some(y);
                ok_line(&mut lines, "heading (handler only)", true, format!("heading +{y:#x} from the 0x00D handler"));
            }
        }
    }
    if let Some(h) = &epa {
        let v = |n: &str| h.get(n).unwrap();
        let x = v("x");
        let (y, z) = (v("add") + v("y_rel"), v("add") + v("z_rel"));
        let good = y == x + 4 && z == x + 8;
        match pos_off {
            Some(p) => ok_line(&mut lines, "position cross-check", good && p == x, format!("0x00D handler writes x,y,z at +{x:#x},+{y:#x},+{z:#x}, constructor says +{p:#x}")),
            None => {
                if good {
                    pos_off = Some(x);
                }
                ok_line(&mut lines, "position (handler only)", good, format!("position +{x:#x},+{y:#x},+{z:#x} from the 0x00D handler"));
            }
        }
    }

    // Projection and distance constants.
    let pj = if sig::resolve(&PROJECTION, image, &ranges).is_ok() { run(&PROJECTION, &mut lines) } else { run(&PROJECTION_FIELD, &mut lines) };
    let dn = run(&DIST_MIN, &mut lines);
    let dx = run(&DIST_MAX, &mut lines);
    let (mut fnum, mut fden, mut slot) = (None, None, None);
    if let Some(h) = &pj {
        let num_rva = to_rva(h.get("num").unwrap());
        let den = if h.sig == "projection_field" {
            let g = to_rva(h.get("g").unwrap());
            let off = h.get("off").unwrap();
            if g.is_some() && g == cam_global_rva && off < 0x10000 {
                g.map(|global_rva| FovDen::Field { global_rva, off })
            } else {
                ok_line(&mut lines, "projection", false, "denominator global is not the camera's game-state global".into());
                None
            }
        } else {
            to_rva(h.get("den").unwrap()).map(FovDen::Const)
        };
        match (num_rva, den) {
            (Some(n), Some(d)) => {
                fnum = Some(n);
                fden = Some(d);
                slot = h.get("slot");
                let sl = h.get("slot").unwrap();
                match (f64_at(image, n), d) {
                    (Some(a), FovDen::Const(_)) => match fov_den_static(image, d) {
                        Some(b) if b != 0.0 => {
                            let f = fov_deg(a, b);
                            let sane = (20.0..120.0).contains(&f);
                            ok_line(&mut lines, "projection", sane, format!("fov = 2*atan({a}/{b}) = {f:.3} deg, SetTransform vtable slot {sl:#x} (state 3)"));
                        }
                        _ => ok_line(&mut lines, "projection", false, "constants unreadable in the image".into()),
                    },
                    (Some(a), FovDen::Field { .. }) => {
                        let sane = a.is_finite() && a > 0.0;
                        ok_line(&mut lines, "projection", sane, format!("fov = 2*atan({a}/den), den = {}, SetTransform vtable slot {sl:#x} (state 3); value is read live", d.describe()));
                    }
                    _ => ok_line(&mut lines, "projection", false, "numerator unreadable in the image".into()),
                }
            }
            _ => ok_line(&mut lines, "projection", false, "constant addresses outside the image".into()),
        }
    }
    let dist = |h: &Option<sig::Hit>, name: &str, lines: &mut Vec<CheckLine>| -> Option<u32> {
        let h = h.as_ref()?;
        let (a, b) = (h.get("cmp").unwrap(), h.get("sub").unwrap());
        if a != b {
            lines.push(CheckLine { name: name.into(), ok: false, detail: format!("compare and subtract constants differ ({a:#x} vs {b:#x})") });
            return None;
        }
        let r = to_rva(a);
        match r.and_then(|r| f32_at(image, r).map(|v| (r, v))) {
            Some((r, v)) => {
                lines.push(CheckLine { name: name.into(), ok: v.is_finite() && v > 0.0, detail: format!("constant at RVA {r:#x} = {v}") });
                Some(r)
            }
            None => {
                lines.push(CheckLine { name: name.into(), ok: false, detail: "constant outside the image".into() });
                None
            }
        }
    };
    let dmin = dist(&dn, "distance minimum", &mut lines);
    let dmax = dist(&dx, "distance maximum", &mut lines);

    let layout = (|| {
        let (s, e, a) = speeds?;
        Some(Layout {
            cam_global_rva: cam_global_rva?,
            cam_ptr_off: cam_ptr_off?,
            cam_look_off: cam_look_off?,
            cam_eye_off: cam_eye_off?,
            ent_table_rva: table_rva?,
            ent_selector_rva: sel_rva?,
            ent_speed_server_off: s,
            ent_speed_eff_off: e,
            ent_speed_aux_off: a,
            ent_pos_off: pos_off?,
            ent_yaw_off: yaw_off?,
            ent_id_off: id_off,
            ent_name_off: name_off,
            fov_num_rva: fnum?,
            fov_den: fden?,
            set_transform_slot: slot?,
            dist_min_rva: dmin?,
            dist_max_rva: dmax?,
        })
    })();
    Resolution { layout, lines }
}

/// What the 2019 Steam-depot build resolves to. After the patch these are expected to change; the
/// check is for the offline baseline and for spotting what moved.
pub fn expected_2019() -> Layout {
    Layout {
        cam_global_rva: 0x4444e4,
        cam_ptr_off: 0x50,
        cam_look_off: 0x50,
        cam_eye_off: 0x44,
        ent_table_rva: 0x46e5f8,
        ent_selector_rva: 0x473a72,
        ent_speed_server_off: 0x158,
        ent_speed_eff_off: 0x98,
        ent_speed_aux_off: 0x9c,
        ent_pos_off: 0x04,
        ent_yaw_off: 0x18,
        ent_id_off: Some(0x78),
        ent_name_off: Some(0x7c),
        fov_num_rva: 0x31ebb0,
        fov_den: FovDen::Const(0x31fef0),
        set_transform_slot: 0x94,
        dist_min_rva: 0x31eba8,
        dist_max_rva: 0x31f258,
    }
}

/// Compares a resolved layout with `want` field by field.
pub fn diff_layout(got: &Layout, want: &Layout) -> Vec<CheckLine> {
    let mut v = Vec::new();
    macro_rules! f {
        ($($field:ident),*) => {$(
            let ok = got.$field == want.$field;
            v.push(CheckLine { name: format!("expect {}", stringify!($field)), ok, detail: format!("{:x?} (expected {:x?})", got.$field, want.$field) });
        )*};
    }
    f!(cam_global_rva, cam_ptr_off, cam_look_off, cam_eye_off, ent_table_rva, ent_selector_rva, ent_speed_server_off, ent_speed_eff_off, ent_speed_aux_off, ent_pos_off, ent_yaw_off, ent_id_off, ent_name_off, fov_num_rva, fov_den, set_transform_slot, dist_min_rva, dist_max_rva);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 30261102_1 projection setup, from the game-state load through the SetTransform call.
    const NEW_SHAPE: &str = "a1 ac 7a 45 10 d9 81 94 95 03 00 d9 80 e0 02 00 00 d8 c1 d9 5c 24 0c d9 80 dc 02 00 00 d8 c1 d9 5c 24 08 dd d8 d9 80 e8 02 00 00 d8 88 f0 02 00 00 d8 3d 1c a6 32 10 d9 5c 24 04 dd 05 40 ad 32 10 d9 80 f4 02 00 00 d9 f3 dc c0 d9 1c 24 52 e8 a9 2a 29 00 a1 58 d1 47 10 8d 54 24 18 52 6a 03 8b 80 bc 0e 00 00 50 8b 08 ff 91 94 00 00 00";

    fn bytes(s: &str) -> Vec<u8> {
        s.split(' ').map(|b| u8::from_str_radix(b, 16).unwrap()).collect()
    }

    #[test]
    fn new_shape_projection_resolves_field() {
        let mut img = vec![0u8; 8];
        img.extend(bytes(NEW_SHAPE));
        img.extend([0u8; 8]);
        let n = img.len() as u32;
        let h = sig::resolve(&PROJECTION_FIELD, &img, &[(0, n)]).unwrap();
        assert_eq!(h.at, vec![8]);
        assert_eq!(h.get("g"), Some(0x10457aac));
        assert_eq!(h.get("num"), Some(0x1032ad40));
        assert_eq!(h.get("off"), Some(0x2f4));
        assert_eq!(h.get("slot"), Some(0x94));
        // The old-shape signature must not match the new code.
        assert!(sig::resolve(&PROJECTION, &img, &[(0, n)]).is_err());
    }

    #[test]
    fn old_shape_projection_resolves_constants() {
        let mut img = vec![0u8; 4];
        img.extend(bytes("dd 05 b0 eb 71 00 dd 05 f0 fe 71 00 d9 f3 8b c8 dc c0 d9 1c 24 53 e8 01 02 03 04 50 6a 03 56 ff 90 94 00 00 00"));
        let n = img.len() as u32;
        let h = sig::resolve(&PROJECTION, &img, &[(0, n)]).unwrap();
        assert_eq!((h.get("num"), h.get("den"), h.get("slot")), (Some(0x71ebb0), Some(0x71fef0), Some(0x94)));
        assert!(sig::resolve(&PROJECTION_FIELD, &img, &[(0, n)]).is_err());
    }

    #[test]
    fn fov_math_and_den_kinds() {
        assert!((fov_deg(192.0, 350.0) - 57.5).abs() < 0.1);
        let c = FovDen::Const(0x31fef0);
        let f = FovDen::Field { global_rva: 0x457aac, off: 0x2f4 };
        assert_ne!(c, f);
        let mut img = vec![0u8; 0x31fef8];
        img[0x31fef0..0x31fef8].copy_from_slice(&350.0f64.to_le_bytes());
        assert_eq!(fov_den_static(&img, c), Some(350.0));
        assert_eq!(fov_den_static(&img, f), None);
        assert!(f.describe().contains("0x457aac"));
    }

    /// Resolves real unpacked images named by `SIGNET_PARITY_IMAGE_2019` (the 2019 build) and
    /// `SIGNET_PARITY_IMAGE_CURRENT` (a current build). Skipped for each variable that is unset
    /// or points at a missing file.
    #[test]
    fn real_images_resolve_when_present() {
        for (var, want_field) in [("SIGNET_PARITY_IMAGE_2019", false), ("SIGNET_PARITY_IMAGE_CURRENT", true)] {
            let Some(path) = std::env::var_os(var) else { continue };
            let path = path.to_string_lossy().into_owned();
            let path = path.as_str();
            let Ok(mut image) = std::fs::read(path) else { continue };
            let pe = crate::pe::parse(&image).unwrap();
            image.resize(image.len().max(pe.size_of_image as usize), 0);
            let res = resolve_layout(&image, &pe, pe.image_base);
            assert!(res.ok(), "{path}: {:?}", res.lines.iter().filter(|l| !l.ok).collect::<Vec<_>>());
            let l = res.layout.unwrap();
            assert_eq!(matches!(l.fov_den, FovDen::Field { .. }), want_field, "{path}");
            if want_field {
                assert_eq!(l.fov_den, FovDen::Field { global_rva: l.cam_global_rva, off: 0x2f4 });
            } else {
                assert_eq!(l, expected_2019());
            }
        }
    }
}
