//! Third-person play mode: fixed-step character movement over zone collision, a follow camera
//! with wall collision, and idle/run animation blending. The simulation (`Sim`) is pure and
//! backend-free so the window loop and the scripted headless run drive the same code.
//!
//! Conventions: FFXI's y axis points down, so "up" is -y and a surface with a smaller y is higher.
//! Yaw 0 faces +z; the facing vector is (sin yaw, cos yaw).
use signet_dat::collision::Collision;
use signet_dat::skeleton::Pose;

use crate::actor::{self, ActorModel, Bone3x4, FRAME_RATE};

/// Simulation rate, independent of the frame rate.
pub const STEP_HZ: f64 = 60.0;
pub const DT: f32 = (1.0 / STEP_HZ) as f32;
/// Body radius used to keep clear of walls (yalms).
pub const RADIUS: f32 = 0.5;
/// Wall probe height above the feet (about knee height).
pub const KNEE: f32 = 0.6;
/// Ground probe starts this far above the feet, so the character can step up small ledges.
pub const PROBE_UP: f32 = 1.0;
/// Steepest walkable slope. Tops at 45 degrees and under are ramps and hills the original
/// game lets you climb; cliff faces and stair sides are well past 60. 50 degrees leaves a
/// margin above 45 for noise in the triangle normals, and stays below the cliffs.
pub const MAX_SLOPE_DEG: f32 = 50.0;
pub const GRAVITY: f32 = 40.0;
/// Seconds to cross-fade between idle and run.
pub const BLEND_TIME: f32 = 0.15;
/// Retail base run speed in yalms/s with no movement-speed gear or buffs. Source: BG-Wiki
/// "Movement Speed" (https://bg-wiki.com/ffxi/Movement_Speed); it was 4.0 before Dec 2013.
pub const BASE_RUN_SPEED: f32 = 5.0;
/// Stride speed (yalms/s) assumed when the run cycle's stride cannot be measured. This is only
/// used to scale the run animation, so it equals the base speed (rate 1.0).
pub const FALLBACK_STRIDE_SPEED: f32 = BASE_RUN_SPEED;
/// Engineering bounds on the movement-speed modifier, in percent (not retail-measured).
pub const MIN_SPEED_PCT: i32 = -50;
pub const MAX_SPEED_PCT: i32 = 100;

/// Rounds a requested movement-speed modifier to the nearest even percent (retail modifiers only
/// come in even percentages; ties such as 13 round away from zero to 14) and clamps it to
/// `MIN_SPEED_PCT..=MAX_SPEED_PCT`.
pub fn even_speed_pct(pct: f32) -> i32 {
    (((pct / 2.0).round() as i32) * 2).clamp(MIN_SPEED_PCT, MAX_SPEED_PCT)
}

/// Run speed in yalms/s for a movement-speed modifier in percent: `BASE_RUN_SPEED * (1 + pct/100)`,
/// e.g. +12 gives 5.6.
pub fn run_speed_for(pct: i32) -> f32 {
    BASE_RUN_SPEED * (1.0 + even_speed_pct(pct as f32) as f32 / 100.0)
}

/// Run-animation playback rate that keeps the planted foot still: movement speed divided by the
/// speed the animation's stride covers at rate 1.
pub fn run_anim_rate(run_speed: f32, stride_speed: f32) -> f32 {
    if stride_speed > 1e-3 { run_speed / stride_speed } else { 1.0 }
}
pub const TURN_DEG_PER_S: f32 = 720.0;
/// Camera look-at height above the feet.
pub const HEAD: f32 = 1.5;

// Retail camera constants. Source of the distance range and FOV: static analysis of FFXiMain.dll
// 2019 build.
// That build is old, so every value needs live re-verification on the current build.
/// Retail camera distance range in yalms (static analysis of FFXiMain.dll 2019 build: clamp 3.0..6.0).
pub const RETAIL_DIST_MIN: f32 = 3.0;
pub const RETAIL_DIST_MAX: f32 = 6.0;
/// Retail starting distance: the far end of the range.
pub const RETAIL_DIST_START: f32 = RETAIL_DIST_MAX;
/// Retail vertical FOV in degrees: 2*atan(192/350) = 57.5 (static analysis of FFXiMain.dll 2019
/// build).
/// ASSUMPTION: this is the vertical FOV (192/350 matches a 384-unit-high, 4:3-era viewport).
pub const RETAIL_FOV_DEG: f32 = 57.5;
/// Retail pitch limits in degrees (negative looks down).
// UNVERIFIED: pending live measurement
pub const RETAIL_PITCH_MIN: f32 = -60.0;
// UNVERIFIED: pending live measurement
pub const RETAIL_PITCH_MAX: f32 = 15.0;
/// Retail look-at height above the feet, yalms.
// UNVERIFIED: pending live measurement
pub const RETAIL_HEAD: f32 = 1.5;
/// Orbit rate, degrees per input unit (mouse pixel). Same for both modes today.
// UNVERIFIED: pending live measurement
pub const ORBIT_DEG_PER_UNIT: f32 = 0.25;
/// Zoom rate, yalms per wheel notch.
// UNVERIFIED: pending live measurement
pub const ZOOM_YALMS_PER_NOTCH: f32 = 1.0;
/// Initial camera pitch in degrees.
// UNVERIFIED: pending live measurement
pub const START_PITCH: f32 = -15.0;
/// Camera orbit rate for the arrow keys (yaw and pitch), degrees per second.
// UNVERIFIED: pending live measurement
pub const ARROW_ORBIT_DEG_PER_S: f32 = 90.0;
/// Free-camera values (the old modern-style camera, unchanged).
pub const FREE_FOV_DEG: f32 = 60.0;
pub const FREE_DIST_MIN: f32 = 1.5;
pub const FREE_DIST_MAX: f32 = 25.0;
pub const FREE_DIST_START: f32 = 8.0;
pub const FREE_PITCH_MIN: f32 = -80.0;
pub const FREE_PITCH_MAX: f32 = 25.0;

/// Follow-camera behavior selected by `--camera-mode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CameraMode {
    /// Retail-style follow camera.
    // Distance range and FOV come from static analysis (see the constants above); pitch limits,
    // look-at height and orbit/zoom rates are still UNVERIFIED.
    #[default]
    Retail,
    /// Free orbit camera with the current placeholder numbers.
    Free,
}

impl CameraMode {
    /// Vertical FOV in degrees.
    pub fn fov_deg(self) -> f32 {
        match self { CameraMode::Retail => RETAIL_FOV_DEG, CameraMode::Free => FREE_FOV_DEG }
    }
    /// (min, max) camera distance in yalms.
    pub fn dist_range(self) -> (f32, f32) {
        match self {
            CameraMode::Retail => (RETAIL_DIST_MIN, RETAIL_DIST_MAX),
            CameraMode::Free => (FREE_DIST_MIN, FREE_DIST_MAX),
        }
    }
    pub fn pitch_range(self) -> (f32, f32) {
        match self {
            CameraMode::Retail => (RETAIL_PITCH_MIN, RETAIL_PITCH_MAX),
            CameraMode::Free => (FREE_PITCH_MIN, FREE_PITCH_MAX),
        }
    }
    pub fn dist_start(self) -> f32 {
        match self { CameraMode::Retail => RETAIL_DIST_START, CameraMode::Free => FREE_DIST_START }
    }
    /// Look-at height above the feet.
    pub fn head(self) -> f32 {
        match self { CameraMode::Retail => RETAIL_HEAD, CameraMode::Free => HEAD }
    }
}

/// What the movement code needs from the zone.
pub trait World {
    /// First surface at or below `y` (down), as in `Collision::ground_height_below`.
    fn ground_below(&self, x: f32, y: f32, z: f32) -> Option<f32>;
    /// Nearest hit along a ray: (distance, unit normal facing the origin).
    fn raycast(&self, origin: [f32; 3], dir: [f32; 3], max: f32) -> Option<(f32, [f32; 3])>;
}

impl World for Collision {
    fn ground_below(&self, x: f32, y: f32, z: f32) -> Option<f32> {
        self.ground_height_below(x, y, z)
    }
    fn raycast(&self, o: [f32; 3], d: [f32; 3], max: f32) -> Option<(f32, [f32; 3])> {
        Collision::raycast(self, o, d, max).map(|h| (h.t, h.normal))
    }
}

/// Turns elapsed real time into whole simulation steps, keeping the remainder.
pub struct FixedStep {
    acc: f64,
}
impl FixedStep {
    pub fn new() -> Self {
        Self { acc: 0.0 }
    }
    /// Adds `dt` seconds (clamped to 0.25 so a stall does not cause a spiral) and returns the
    /// number of steps to run.
    pub fn advance(&mut self, dt: f64) -> u32 {
        self.acc += dt.clamp(0.0, 0.25);
        let step = 1.0 / STEP_HZ;
        let mut n = 0;
        while self.acc >= step - 1e-9 {
            self.acc -= step;
            n += 1;
        }
        n
    }
    /// How far into the next step the clock is, 0..1, for render interpolation.
    pub fn alpha(&self) -> f32 {
        ((self.acc * STEP_HZ) as f32).clamp(0.0, 1.0)
    }
}

/// Idle/run cross-fade: `run` is the run weight, 0 = idle, 1 = run.
#[derive(Clone, Copy, Debug)]
pub struct AnimBlend {
    pub run: f32,
}
impl AnimBlend {
    pub fn update(&mut self, moving: bool, dt: f32) {
        let target = if moving { 1.0 } else { 0.0 };
        let d = dt / BLEND_TIME;
        self.run = if self.run < target { (self.run + d).min(target) } else { (self.run - d).max(target) };
    }
    /// (idle weight, run weight); always sums to 1.
    pub fn weights(&self) -> (f32, f32) {
        (1.0 - self.run, self.run)
    }
}

fn hlen(v: [f32; 2]) -> f32 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

/// Whether a rise of `rise` yalms (positive = uphill) over `run` horizontal yalms is too steep.
pub fn too_steep(rise: f32, run: f32) -> bool {
    rise > run * MAX_SLOPE_DEG.to_radians().tan() + 0.02
}

#[derive(Clone, Copy, Debug)]
pub struct Player {
    pub pos: [f32; 3],
    pub vy: f32,
    pub yaw: f32,
}

/// Applies a horizontal move against walls: a knee-height ray along the move finds a wall, the
/// move is projected onto the wall plane (slide), and the body is kept `RADIUS` off the wall.
/// Returns the new xz.
pub fn move_horizontal(w: &dyn World, pos: [f32; 3], delta: [f32; 2]) -> [f32; 2] {
    if hlen(delta) < 1e-6 {
        return [pos[0], pos[2]];
    }
    let mut d = delta;
    // Up to two wall contacts (a corner): project the move onto each wall plane in turn.
    for _ in 0..2 {
        let l = hlen(d);
        if l < 1e-6 {
            break;
        }
        let u = [d[0] / l, d[1] / l];
        let Some((t, n)) = w.raycast([pos[0], pos[1] - KNEE, pos[2]], [u[0], 0.0, u[1]], (l + RADIUS) * 4.0) else { break };
        // A mostly-vertical normal is ground we can walk on, not a wall.
        if n[1].abs() > MAX_SLOPE_DEG.to_radians().cos() {
            break;
        }
        let nl = hlen([n[0], n[2]]);
        if nl < 1e-4 {
            break;
        }
        let nh = [n[0] / nl, n[2] / nl];
        // How much of the move heads into the wall, and the room left before the body touches it.
        let into = -(d[0] * nh[0] + d[1] * nh[1]);
        if into <= 0.0 {
            break;
        }
        let perp = t * -(u[0] * nh[0] + u[1] * nh[1]);
        let allowed = perp - RADIUS; // negative: already too close, push out
        let along = into.min(allowed);
        let tang = [d[0] + nh[0] * into, d[1] + nh[1] * into];
        d = [tang[0] - nh[0] * along, tang[1] - nh[1] * along];
    }
    [pos[0] + d[0], pos[2] + d[1]]
}

impl Player {
    /// One simulation step. `dir` is the desired world-space unit move direction (or zero) and
    /// `speed` yalms per second. Returns true if the character moved horizontally.
    pub fn step(&mut self, w: &dyn World, dir: [f32; 2], speed: f32, dt: f32) -> bool {
        let mut moved = false;
        let delta = [dir[0] * speed * dt, dir[1] * speed * dt];
        if hlen(delta) > 0.0 {
            // The wall-resolved move first, then each axis alone as a fallback slide when the
            // slope blocks the full move.
            let tries = [delta, [delta[0], 0.0], [0.0, delta[1]]];
            for d in tries {
                let c = move_horizontal(w, self.pos, d);
                let step = hlen([c[0] - self.pos[0], c[1] - self.pos[2]]);
                if step < 1e-6 {
                    continue;
                }
                if let Some(g) = w.ground_below(c[0], self.pos[1] - PROBE_UP, c[1]) {
                    if !too_steep(self.pos[1] - g, step) {
                        self.pos[0] = c[0];
                        self.pos[2] = c[1];
                        moved = true;
                        break;
                    }
                }
            }
        }
        // Vertical: snap up to ground, or fall toward it.
        if let Some(g) = w.ground_below(self.pos[0], self.pos[1] - PROBE_UP, self.pos[2]) {
            if g <= self.pos[1] + 1e-4 {
                self.pos[1] = g;
                self.vy = 0.0;
            } else {
                self.vy += GRAVITY * dt;
                self.pos[1] = (self.pos[1] + self.vy * dt).min(g);
                if self.pos[1] >= g {
                    self.vy = 0.0;
                }
            }
        }
        moved
    }
}

pub fn shortest_turn(from: f32, to: f32) -> f32 {
    (to - from + 540.0).rem_euclid(360.0) - 180.0
}

/// Keys and mouse for one simulation step.
#[derive(Clone, Copy, Default, Debug)]
pub struct Input {
    pub fwd: bool,
    pub back: bool,
    pub left: bool,
    pub right: bool,
    pub dx: f32,
    pub dy: f32,
    pub wheel: f32,
    /// Arrow keys that orbit the camera.
    pub cam_left: bool,
    pub cam_right: bool,
    pub cam_up: bool,
    pub cam_down: bool,
}

pub struct Sim {
    pub player: Player,
    pub prev_pos: [f32; 3],
    pub cam_yaw: f32,
    pub cam_pitch: f32,
    pub cam_dist: f32,
    pub blend: AnimBlend,
    pub idle_t: f32,
    pub run_t: f32,
    pub run_speed: f32,
    /// Planted-foot speed of the run animation at rate 1 (measured).
    pub stride_speed: f32,
    pub moving: bool,
    pub ticks: u64,
    pub camera_mode: CameraMode,
}

impl Sim {
    pub fn new(pos: [f32; 3], yaw: f32, run_speed: f32, stride_speed: f32) -> Self {
        Sim {
            player: Player { pos, vy: 0.0, yaw },
            prev_pos: pos,
            cam_yaw: yaw,
            cam_pitch: START_PITCH,
            cam_dist: CameraMode::default().dist_start(),
            blend: AnimBlend { run: 0.0 },
            idle_t: 0.0,
            run_t: 0.0,
            run_speed,
            stride_speed,
            moving: false,
            ticks: 0,
            camera_mode: CameraMode::default(),
        }
    }

    /// Switch camera mode and reset the zoom to that mode's starting distance.
    pub fn set_camera_mode(&mut self, m: CameraMode) {
        self.camera_mode = m;
        self.cam_dist = m.dist_start();
        let (lo, hi) = m.pitch_range();
        self.cam_pitch = self.cam_pitch.clamp(lo, hi);
    }

    /// Camera-relative move direction for the keys held (unit or zero).
    pub fn wish_dir(&self, i: &Input) -> [f32; 2] {
        let (s, c) = self.cam_yaw.to_radians().sin_cos();
        // Same as Camera::right in xz: cross(up, forward) for forward (sin, cos).
        let (f, r) = ([s, c], [-c, s]);
        let a = (i.fwd as i32 - i.back as i32) as f32;
        let b = (i.right as i32 - i.left as i32) as f32;
        let v = [f[0] * a + r[0] * b, f[1] * a + r[1] * b];
        let l = hlen(v);
        if l > 1e-6 { [v[0] / l, v[1] / l] } else { [0.0, 0.0] }
    }

    pub fn tick(&mut self, w: &dyn World, i: &Input) {
        let (plo, phi) = self.camera_mode.pitch_range();
        let (dlo, dhi) = self.camera_mode.dist_range();
        let (arrow_x, arrow_y) = ((i.cam_right as i32 - i.cam_left as i32) as f32, (i.cam_up as i32 - i.cam_down as i32) as f32);
        self.cam_yaw += i.dx * ORBIT_DEG_PER_UNIT + arrow_x * ARROW_ORBIT_DEG_PER_S * DT;
        self.cam_pitch = (self.cam_pitch - i.dy * ORBIT_DEG_PER_UNIT + arrow_y * ARROW_ORBIT_DEG_PER_S * DT).clamp(plo, phi);
        self.cam_dist = (self.cam_dist - i.wheel * ZOOM_YALMS_PER_NOTCH).clamp(dlo, dhi);
        let dir = self.wish_dir(i);
        let wants = hlen(dir) > 0.0;
        self.prev_pos = self.player.pos;
        self.moving = self.player.step(w, dir, if wants { self.run_speed } else { 0.0 }, DT) && wants;
        if wants {
            let target = dir[0].atan2(dir[1]).to_degrees();
            let turn = shortest_turn(self.player.yaw, target);
            let maxt = TURN_DEG_PER_S * DT;
            self.player.yaw = (self.player.yaw + turn.clamp(-maxt, maxt)).rem_euclid(360.0);
        }
        self.blend.update(self.moving, DT);
        self.idle_t += DT;
        if self.blend.run > 0.0 {
            self.run_t += DT * run_anim_rate(self.run_speed, self.stride_speed);
        }
        self.ticks += 1;
    }

    /// Follow camera for a character at `feet`; pulled in front of walls.
    pub fn camera(&self, w: &dyn World, feet: [f32; 3]) -> crate::Camera {
        match self.camera_mode {
            CameraMode::Retail => self.camera_retail(w, feet),
            CameraMode::Free => self.camera_free(w, feet),
        }
    }

    /// Retail camera: distance, pitch and look-at height use the retail constants.
    fn camera_retail(&self, w: &dyn World, feet: [f32; 3]) -> crate::Camera {
        self.camera_with(CameraMode::Retail, w, feet)
    }

    fn camera_free(&self, w: &dyn World, feet: [f32; 3]) -> crate::Camera {
        self.camera_with(CameraMode::Free, w, feet)
    }

    /// Eye placed behind the look-at point, pulled in front of walls (both modes).
    fn camera_with(&self, m: CameraMode, w: &dyn World, feet: [f32; 3]) -> crate::Camera {
        let target = [feet[0], feet[1] - m.head(), feet[2]];
        let (plo, phi) = m.pitch_range();
        let (dlo, dhi) = m.dist_range();
        let pitch = self.cam_pitch.clamp(plo, phi);
        let cam = crate::Camera { pos: [0.0; 3], yaw: self.cam_yaw, pitch };
        let f = cam.forward();
        let mut dist = self.cam_dist.clamp(dlo, dhi);
        if let Some((t, _)) = w.raycast(target, [-f[0], -f[1], -f[2]], dist + 0.4) {
            dist = (t - 0.4).clamp(0.4, dist);
        }
        crate::Camera { pos: [target[0] - f[0] * dist, target[1] - f[1] * dist, target[2] - f[2] * dist], yaw: self.cam_yaw, pitch }
    }

    /// One row of the retail parity log (`--parity-log`) for the tick that just ran with input `i`.
    /// Same columns as the retail probe; positions are the feet, the look-at point is the feet
    /// raised by the mode's look-at height, and the eye is the (wall-pulled) camera position.
    /// Signet orbits with the mouse without a button, so a mouse move is logged as the right button.
    pub fn parity_row(&self, w: &dyn World, i: &Input) -> signet_parity::rows::Row {
        use signet_parity::rows::{Keys, Row};
        let feet = self.player.pos;
        let cam = self.camera(w, feet);
        let look = [feet[0], feet[1] - self.camera_mode.head(), feet[2]];
        let f = |v: [f32; 3]| [v[0] as f64, v[1] as f64, v[2] as f64];
        let mut r = Row {
            t: self.ticks as f64 / STEP_HZ,
            pos: f(feet),
            heading_deg: self.player.yaw.rem_euclid(360.0) as f64,
            speed_eff: self.run_speed as f64,
            speed_srv: self.run_speed as f64,
            eye: f(cam.pos),
            look: f(look),
            fov_deg: self.camera_mode.fov_deg() as f64,
            keys: Keys { w: i.fwd, a: i.left, s: i.back, d: i.right, up: i.cam_up, down: i.cam_down, left: i.cam_left, right: i.cam_right, rmb: i.dx != 0.0 || i.dy != 0.0, ..Default::default() },
            wheel: i.wheel.round() as i32,
            mouse_dx: i.dx.round() as i32,
            mouse_dy: i.dy.round() as i32,
            ..Default::default()
        };
        r.derive();
        r
    }

    pub fn render_pos(&self, alpha: f32) -> [f32; 3] {
        actor::lerp3(self.prev_pos, self.player.pos, alpha)
    }
}

/// Blends two local pose sets bone by bone (slerp rotation, lerp translation).
pub fn blend_poses(a: &[Pose], b: &[Pose], t: f32) -> Vec<Pose> {
    a.iter()
        .zip(b)
        .map(|(x, y)| Pose { rotation: actor::slerp(x.rotation, y.rotation, t), translation: actor::lerp3(x.translation, y.translation, t) })
        .collect()
}

/// Palette for the idle/run blend, placed at `pos` facing the player's yaw.
pub fn blended_palette(m: &ActorModel, idle: &str, run: &str, s: &Sim, pos: [f32; 3]) -> Vec<Bone3x4> {
    let place = actor::placement(pos, s.player.yaw);
    let pose = |name: &str, t: f32| match m.animation(name) {
        Some(a) => actor::local_poses_at(a, &m.bind_local, t),
        None => m.bind_local.clone(),
    };
    let li = pose(idle, s.idle_t);
    let (_, wr) = s.blend.weights();
    let local = if wr > 0.0 { blend_poses(&li, &pose(run, s.run_t), wr) } else { li };
    let world: Vec<Pose> = m.skeleton.world_from_local(&local).iter().map(|w| place.compose(w)).collect();
    actor::palette(&world)
}

/// Run speed in yalms/s that keeps the planted foot still on the ground. In each frame the
/// lowest bone is the planted foot; the horizontal distance it travels to the next frame (model
/// space, body fixed) is how far the body must move in that time. The median over the loop
/// ignores the frames where the lowest bone swaps feet.
pub fn measure_stride(m: &ActorModel, name: &str) -> Option<f32> {
    let a = m.animation(name)?;
    let n = a.frames as usize;
    if n < 2 {
        return None;
    }
    // Foot candidates: bones off the body's center line (|z| > 0.1) that sit near the ground in
    // the bind pose. This skips the root and helper bones that sit on the center line at y 0.
    let bind = m.skeleton.bind_world();
    let lateral: Vec<usize> = (0..bind.len()).filter(|&i| bind[i].translation[2].abs() > 0.1).collect();
    let floor = lateral.iter().map(|&i| bind[i].translation[1]).fold(f32::MIN, f32::max);
    let feet: Vec<usize> = lateral.into_iter().filter(|&i| bind[i].translation[1] > floor - 0.15).collect();
    if feet.is_empty() {
        return None;
    }
    let step = 1.0 / (FRAME_RATE * a.speed);
    let worlds: Vec<Vec<Pose>> = (0..n)
        .map(|f| {
            let local = actor::local_poses_at(a, &m.bind_local, f as f32 * step);
            m.skeleton.world_from_local(&local)
        })
        .collect();
    let mut speeds = Vec::new();
    for f in 0..n {
        let (w0, w1) = (&worlds[f], &worlds[(f + 1) % n]);
        let low = feet.iter().copied().max_by(|&i, &j| w0[i].translation[1].total_cmp(&w0[j].translation[1]))?;
        let (p0, p1) = (w0[low].translation, w1[low].translation);
        speeds.push(hlen([p1[0] - p0[0], p1[2] - p0[2]]) / step);
    }
    speeds.sort_by(|x, y| x.total_cmp(y));
    Some(speeds[speeds.len() / 2])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ground function plus an optional wall plane at x = wall (normal facing -x).
    struct Mock {
        ground: fn(f32, f32) -> f32,
        wall: Option<f32>,
    }
    impl World for Mock {
        fn ground_below(&self, x: f32, _y: f32, z: f32) -> Option<f32> {
            Some((self.ground)(x, z))
        }
        fn raycast(&self, o: [f32; 3], d: [f32; 3], max: f32) -> Option<(f32, [f32; 3])> {
            let wx = self.wall?;
            if d[0] <= 1e-6 {
                return None;
            }
            let t = (wx - o[0]) / d[0];
            (t >= 0.0 && t <= max).then_some((t, [-1.0, 0.0, 0.0]))
        }
    }
    fn flat(_: f32, _: f32) -> f32 {
        0.0
    }

    #[test]
    fn slides_along_a_wall_plane() {
        let w = Mock { ground: flat, wall: Some(5.0) };
        let mut p = Player { pos: [0.0, 0.0, 0.0], vy: 0.0, yaw: 0.0 };
        for _ in 0..600 {
            p.step(&w, [0.70711, 0.70711], 5.0, DT);
        }
        assert!(p.pos[0] <= 5.0 - RADIUS + 0.01, "x {}", p.pos[0]);
        assert!(p.pos[0] >= 5.0 - RADIUS - 0.2, "x {}", p.pos[0]);
        assert!(p.pos[2] > 20.0, "z {}", p.pos[2]);
    }

    #[test]
    fn head_on_wall_stops_without_clipping() {
        let w = Mock { ground: flat, wall: Some(5.0) };
        let mut p = Player { pos: [0.0, 0.0, 0.0], vy: 0.0, yaw: 0.0 };
        for _ in 0..600 {
            p.step(&w, [1.0, 0.0], 5.0, DT);
        }
        assert!(p.pos[0] <= 5.0 - RADIUS + 0.01 && p.pos[0] > 4.0, "x {}", p.pos[0]);
        assert!(p.pos[2].abs() < 1e-4);
    }

    #[test]
    fn steep_slope_is_rejected_and_gentle_one_climbed() {
        // 45 degrees (1 yalm up per yalm) is walkable; 63 degrees (2 per yalm) is not.
        let gentle = Mock { ground: |x, _| -x, wall: None };
        let steep = Mock { ground: |x, _| if x < 2.0 { 0.0 } else { -(x - 2.0) * 2.0 }, wall: None };
        let mut a = Player { pos: [0.0, 0.0, 0.0], vy: 0.0, yaw: 0.0 };
        let mut b = a;
        for _ in 0..120 {
            a.step(&gentle, [1.0, 0.0], 5.0, DT);
            b.step(&steep, [1.0, 0.0], 5.0, DT);
        }
        assert!(a.pos[0] > 8.0, "gentle x {}", a.pos[0]);
        assert!((a.pos[1] + a.pos[0]).abs() < 0.2, "feet follow ground: y {} x {}", a.pos[1], a.pos[0]);
        assert!(b.pos[0] < 2.2, "steep x {}", b.pos[0]);
        assert!(too_steep(2.0, 1.0) && !too_steep(1.0, 1.0));
    }

    #[test]
    fn falls_to_lower_ground() {
        let w = Mock { ground: flat, wall: None };
        let mut p = Player { pos: [0.0, -3.0, 0.0], vy: 0.0, yaw: 0.0 };
        p.step(&w, [0.0, 0.0], 0.0, DT);
        assert!(p.pos[1] > -3.0 && p.pos[1] < 0.0, "y {}", p.pos[1]);
        for _ in 0..120 {
            p.step(&w, [0.0, 0.0], 0.0, DT);
        }
        assert_eq!(p.pos[1], 0.0);
    }

    #[test]
    fn fixed_step_accumulates() {
        let mut f = FixedStep::new();
        assert_eq!(f.advance(1.0 / 30.0), 2);
        // ten 10 ms frames = 0.1 s = 6 steps
        let n: u32 = (0..10).map(|_| f.advance(0.01)).sum();
        assert_eq!(n, 6);
        // a huge stall is clamped to 0.25 s = 15 steps
        assert_eq!(f.advance(10.0), 15);
        // the remainder carries: 10 ms is not a step (16.7 ms), two are
        let mut g = FixedStep::new();
        assert_eq!(g.advance(0.01), 0);
        assert_eq!(g.advance(0.01), 1);
        assert!(g.alpha() > 0.0 && g.alpha() < 1.0);
    }

    #[test]
    fn blend_weights_ramp_over_the_blend_time() {
        let mut b = AnimBlend { run: 0.0 };
        b.update(true, BLEND_TIME / 2.0);
        let (i, r) = b.weights();
        assert!((r - 0.5).abs() < 1e-5 && (i + r - 1.0).abs() < 1e-6);
        b.update(true, BLEND_TIME);
        assert_eq!(b.weights(), (0.0, 1.0));
        b.update(false, BLEND_TIME / 4.0);
        assert!((b.run - 0.75).abs() < 1e-5);
        for _ in 0..100 {
            b.update(false, DT);
        }
        assert_eq!(b.weights(), (1.0, 0.0));
    }

    #[test]
    fn turn_takes_the_short_way() {
        assert!((shortest_turn(350.0, 10.0) - 20.0).abs() < 1e-4);
        assert!((shortest_turn(10.0, 350.0) + 20.0).abs() < 1e-4);
    }

    #[test]
    fn wish_dir_is_camera_relative() {
        let s = Sim::new([0.0; 3], 0.0, 5.0, 4.19);
        let f = s.wish_dir(&Input { fwd: true, ..Default::default() });
        assert!(f[0].abs() < 1e-5 && (f[1] - 1.0).abs() < 1e-5);
        let r = s.wish_dir(&Input { right: true, ..Default::default() });
        let cr = crate::Camera { pos: [0.0; 3], yaw: 0.0, pitch: 0.0 }.right();
        assert!((r[0] - cr[0]).abs() < 1e-5 && (r[1] - cr[2]).abs() < 1e-5);
    }

    #[test]
    fn speed_modifier_math() {
        assert_eq!(even_speed_pct(12.0), 12);
        assert_eq!(even_speed_pct(13.0), 14);
        assert_eq!(even_speed_pct(11.0), 12);
        assert_eq!(even_speed_pct(-13.0), -14);
        assert_eq!(even_speed_pct(500.0), MAX_SPEED_PCT);
        assert_eq!(even_speed_pct(-500.0), MIN_SPEED_PCT);
        assert!((run_speed_for(0) - 5.0).abs() < 1e-6);
        assert!((run_speed_for(12) - 5.6).abs() < 1e-5);
        assert!((run_speed_for(25) - 5.0 * 1.26).abs() < 1e-5, "odd percent rounds to even");
    }

    #[test]
    fn run_anim_rate_scales_with_speed() {
        assert!((run_anim_rate(5.0, 4.19) - 1.1933).abs() < 1e-3);
        assert!((run_anim_rate(5.6, 5.6) - 1.0).abs() < 1e-6);
        assert_eq!(run_anim_rate(5.0, 0.0), 1.0);
        // The run clock advances at that rate while moving.
        let w = Mock { ground: flat, wall: None };
        let mut s = Sim::new([0.0; 3], 0.0, 5.0, 4.19);
        s.blend.run = 1.0;
        for _ in 0..60 {
            s.tick(&w, &Input { fwd: true, ..Default::default() });
        }
        assert!((s.run_t - 1.0 * 5.0 / 4.19).abs() < 0.01, "run_t {}", s.run_t);
    }

    fn flat_world() -> Mock {
        Mock { ground: flat, wall: None }
    }

    #[test]
    fn retail_distance_clamped_and_starts_far() {
        let mut s = Sim::new([0.0; 3], 0.0, 5.0, 5.0);
        s.set_camera_mode(CameraMode::Retail);
        assert_eq!(s.cam_dist, 6.0);
        let w = flat_world();
        s.tick(&w, &Input { wheel: 50.0, ..Default::default() });
        assert_eq!(s.cam_dist, 3.0);
        s.tick(&w, &Input { wheel: -50.0, ..Default::default() });
        assert_eq!(s.cam_dist, 6.0);
        s.cam_dist = 20.0; // out-of-range set by a script is clamped when the camera is built
        let c = s.camera(&w, [0.0, 0.0, 0.0]);
        let d = (c.pos[0].powi(2) + (c.pos[1] + RETAIL_HEAD).powi(2) + c.pos[2].powi(2)).sqrt();
        assert!((d - 6.0).abs() < 1e-3, "{d}");
    }

    #[test]
    fn free_distance_range_unchanged() {
        let mut s = Sim::new([0.0; 3], 0.0, 5.0, 5.0);
        s.set_camera_mode(CameraMode::Free);
        assert_eq!(s.cam_dist, 8.0);
        s.tick(&flat_world(), &Input { wheel: -100.0, ..Default::default() });
        assert_eq!(s.cam_dist, 25.0);
    }

    #[test]
    fn fov_per_mode() {
        assert_eq!(CameraMode::Retail.fov_deg(), 57.5);
        assert_eq!(CameraMode::Free.fov_deg(), 60.0);
        let exact = 2.0 * (192.0f32 / 350.0).atan().to_degrees();
        assert!((RETAIL_FOV_DEG - exact).abs() < 0.05, "{exact}");
    }

    #[test]
    fn parity_row_matches_the_sim_camera() {
        let w = flat_world();
        let mut s = Sim::new([3.0, 0.0, -2.0], 0.0, 5.0, 5.0);
        s.cam_pitch = -20.0;
        s.cam_yaw = 30.0;
        s.cam_dist = 4.5;
        let inp = Input { fwd: true, ..Default::default() };
        s.tick(&w, &inp);
        let r = s.parity_row(&w, &inp);
        assert!((r.dist - 4.5).abs() < 1e-4, "dist {}", r.dist);
        assert!((r.cam_pitch + 20.0).abs() < 1e-3, "pitch {}", r.cam_pitch);
        assert!((r.cam_yaw - 30.0).abs() < 1e-3, "yaw {}", r.cam_yaw);
        assert!((r.pos[1] - r.look[1] - RETAIL_HEAD as f64).abs() < 1e-5);
        assert!(r.keys.w && !r.keys.s && (r.t - 1.0 / 60.0).abs() < 1e-9 && (r.fov_deg - 57.5).abs() < 1e-9);
    }

    #[test]
    fn arrow_keys_orbit_at_the_arrow_rate_and_clamp_pitch() {
        let w = flat_world();
        let mut s = Sim::new([0.0; 3], 0.0, 5.0, 5.0);
        let yaw0 = s.cam_yaw;
        for _ in 0..60 {
            s.tick(&w, &Input { cam_right: true, ..Default::default() });
        }
        assert!((s.cam_yaw - yaw0 - ARROW_ORBIT_DEG_PER_S).abs() < 1e-2, "yaw moved {}", s.cam_yaw - yaw0);
        for _ in 0..600 {
            s.tick(&w, &Input { cam_up: true, ..Default::default() });
        }
        assert_eq!(s.cam_pitch, RETAIL_PITCH_MAX);
        for _ in 0..600 {
            s.tick(&w, &Input { cam_down: true, ..Default::default() });
        }
        assert_eq!(s.cam_pitch, RETAIL_PITCH_MIN);
    }

    #[test]
    fn default_mode_is_retail() {
        assert_eq!(CameraMode::default(), CameraMode::Retail);
        assert_eq!(Sim::new([0.0; 3], 0.0, 5.0, 5.0).camera_mode, CameraMode::Retail);
    }
}
