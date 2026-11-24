//! Synthetic recordings with known parameters, following the order of the measurement routine
//! (`docs/RETAIL_MEASUREMENT_ROUTINE.md`). Used to test the comparator: the metrics computed from
//! a generated recording must recover the parameters that generated it. The numbers in
//! `retail_like` are made up for the test and say nothing about the real retail client.

use crate::rows::{Keys, Row, derive_camera};

#[derive(Clone, Debug)]
pub struct Params {
    pub run_speed: f64,
    pub accel_time: f64,
    pub decel_time: f64,
    pub turn_rate: f64,
    pub follow_lag: f64,
    pub dist_min: f64,
    pub dist_max: f64,
    pub zoom_step: f64,
    pub pitch_min: f64,
    pub pitch_max: f64,
    pub start_pitch: f64,
    pub key_yaw_rate: f64,
    pub key_pitch_rate: f64,
    pub mouse_deg_per_count: f64,
    pub look_height: f64,
    pub fov: f64,
}

impl Params {
    pub fn retail_like() -> Params {
        Params {
            run_speed: 5.4,
            accel_time: 0.25,
            decel_time: 0.20,
            turn_rate: 540.0,
            follow_lag: 0.12,
            dist_min: 3.0,
            dist_max: 6.0,
            zoom_step: 0.75,
            pitch_min: -50.0,
            pitch_max: 20.0,
            start_pitch: -12.0,
            key_yaw_rate: 100.0,
            key_pitch_rate: 40.0,
            mouse_deg_per_count: 0.2,
            look_height: 1.6,
            fov: 57.5,
        }
    }
    /// Values that mimic Signet's current placeholders.
    pub fn signet_like() -> Params {
        Params {
            run_speed: 5.0,
            accel_time: 0.02,
            decel_time: 0.02,
            turn_rate: 720.0,
            follow_lag: 0.0,
            dist_min: 3.0,
            dist_max: 6.0,
            zoom_step: 1.0,
            pitch_min: -60.0,
            pitch_max: 15.0,
            start_pitch: -15.0,
            key_yaw_rate: 90.0,
            key_pitch_rate: 90.0,
            mouse_deg_per_count: 0.25,
            look_height: 1.5,
            fov: 57.5,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct In {
    keys: Keys,
    wheel: i32,
    dx: i32,
    dy: i32,
}

fn seg(v: &mut Vec<In>, secs: f64, f: impl Fn(&mut In, usize)) {
    let n = (secs * 60.0).round() as usize;
    for i in 0..n {
        let mut x = In::default();
        f(&mut x, i);
        v.push(x);
    }
}

fn script() -> Vec<In> {
    let mut s = Vec::new();
    seg(&mut s, 3.0, |_, _| {});
    seg(&mut s, 5.0, |x, _| x.keys.w = true);
    seg(&mut s, 3.0, |_, _| {});
    seg(&mut s, 3.0, |x, _| x.keys.left = true);
    seg(&mut s, 3.0, |x, _| x.keys.right = true);
    seg(&mut s, 2.0, |x, _| x.keys.up = true);
    seg(&mut s, 3.0, |x, _| x.keys.down = true);
    seg(&mut s, 1.0, |_, _| {});
    seg(&mut s, 1.5, |x, _| {
        x.keys.rmb = true;
        x.dx = 6;
    });
    seg(&mut s, 1.0, |x, _| {
        x.keys.rmb = true;
        x.dy = -4;
    });
    seg(&mut s, 1.0, |_, _| {});
    for _ in 0..6 {
        seg(&mut s, 1.0, |x, i| x.wheel = if i == 10 { 1 } else { 0 });
    }
    for _ in 0..6 {
        seg(&mut s, 1.0, |x, i| x.wheel = if i == 10 { -1 } else { 0 });
    }
    seg(&mut s, 1.0, |_, _| {});
    seg(&mut s, 2.0, |x, _| x.keys.w = true);
    seg(&mut s, 1.0, |x, _| x.keys.d = true);
    seg(&mut s, 2.0, |x, _| x.keys.w = true);
    seg(&mut s, 1.0, |x, _| x.keys.d = true);
    seg(&mut s, 2.0, |_, _| {});
    s
}

pub fn generate(p: &Params) -> Vec<Row> {
    let dt = 1.0 / 60.0;
    let mut pos = [0.0f64; 3];
    let mut look = [0.0f64, -p.look_height, 0.0];
    let (mut heading, mut v) = (0.0f64, 0.0f64);
    let (mut yaw, mut pitch) = (0.0f64, p.start_pitch);
    let (mut dist, mut dist_target) = (p.dist_max, p.dist_max);
    let mut out = Vec::new();
    let lag_k = if p.follow_lag > 0.0 { 1.0 - (-dt / p.follow_lag).exp() } else { 1.0 };
    for (i, inp) in script().iter().enumerate() {
        let k = &inp.keys;
        // Camera.
        let yaw_dir = k.right as i32 - k.left as i32;
        let pitch_dir = k.up as i32 - k.down as i32;
        yaw += yaw_dir as f64 * p.key_yaw_rate * dt + inp.dx as f64 * p.mouse_deg_per_count;
        pitch = (pitch + pitch_dir as f64 * p.key_pitch_rate * dt - inp.dy as f64 * p.mouse_deg_per_count).clamp(p.pitch_min, p.pitch_max);
        dist_target = (dist_target - inp.wheel as f64 * p.zoom_step).clamp(p.dist_min, p.dist_max);
        dist += (dist_target - dist) * (1.0 - (-dt / 0.05f64).exp());
        // Character: forward is camera-relative; D turns 90 degrees right.
        let moving = k.w || k.d;
        let target_speed = if moving { p.run_speed } else { 0.0 };
        let (a, d) = (p.run_speed / p.accel_time, p.run_speed / p.decel_time);
        v = if v < target_speed { (v + a * dt).min(target_speed) } else { (v - d * dt).max(target_speed) };
        if moving {
            let want = yaw + if k.d { 90.0 } else { 0.0 };
            let turn = (want - heading + 540.0).rem_euclid(360.0) - 180.0;
            let m = p.turn_rate * dt;
            heading += turn.clamp(-m, m);
        }
        let h = heading.to_radians();
        pos[0] += h.sin() * v * dt;
        pos[2] += h.cos() * v * dt;
        // Look-at follows the character with a first-order lag.
        look[0] += (pos[0] - look[0]) * lag_k;
        look[2] += (pos[2] - look[2]) * lag_k;
        look[1] = pos[1] - p.look_height;
        let (yr, pr) = (yaw.to_radians(), pitch.to_radians());
        let f = [pr.cos() * yr.sin(), -pr.sin(), pr.cos() * yr.cos()];
        let eye = [look[0] - f[0] * dist, look[1] - f[1] * dist, look[2] - f[2] * dist];
        let mut r = Row {
            t: i as f64 * dt,
            pos,
            heading_deg: heading.rem_euclid(360.0),
            speed_eff: p.run_speed,
            speed_srv: p.run_speed,
            eye,
            look,
            fov_deg: p.fov,
            keys: *k,
            wheel: inp.wheel,
            mouse_dx: inp.dx,
            mouse_dy: inp.dy,
            ..Default::default()
        };
        let (d, y, pt) = derive_camera(eye, look);
        r.dist = d;
        r.cam_yaw = y;
        r.cam_pitch = pt;
        out.push(r);
    }
    out
}
