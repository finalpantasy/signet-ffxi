//! Retail-versus-Signet metrics from two parity CSVs.
//!
//! Every metric is computed the same way for both recordings, from the rows alone (positions, the
//! camera eye and look-at, and the key, mouse and wheel columns), so a difference is a difference
//! in behavior and not in method. Metrics that need an input the recording does not contain
//! (for example no wheel events) come out as `n/a`.

use crate::rows::{Q_BAD_MASK, Row, NUM_2, NUM_4, NUM_6, NUM_8};
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// One row of the output table: what is measured and which Signet constant it can set.
pub struct Spec {
    pub id: &'static str,
    pub label: &'static str,
    pub unit: &'static str,
    pub decimals: usize,
    /// Signet constant (in `crates/signet-render/src/play.rs`) the retail value should replace.
    pub constant: Option<&'static str>,
}

pub const SPECS: &[Spec] = &[
    Spec { id: "run_speed", label: "steady run speed", unit: "yalms/s", decimals: 3, constant: Some("BASE_RUN_SPEED") },
    Spec { id: "accel_time", label: "acceleration time to 90%", unit: "s", decimals: 3, constant: None },
    Spec { id: "decel_time", label: "deceleration time to 10%", unit: "s", decimals: 3, constant: None },
    Spec { id: "speed_field", label: "effective speed field", unit: "yalms/s", decimals: 3, constant: None },
    Spec { id: "turn_rate", label: "character turn rate (p95)", unit: "deg/s", decimals: 1, constant: Some("TURN_DEG_PER_S") },
    Spec { id: "follow_lag_dist", label: "camera follow lag (distance)", unit: "yalms", decimals: 3, constant: None },
    Spec { id: "follow_lag_time", label: "camera follow lag (time)", unit: "s", decimals: 3, constant: None },
    Spec { id: "dist_min", label: "camera distance minimum (p2)", unit: "yalms", decimals: 3, constant: Some("RETAIL_DIST_MIN") },
    Spec { id: "dist_max", label: "camera distance maximum (p98)", unit: "yalms", decimals: 3, constant: Some("RETAIL_DIST_MAX") },
    Spec { id: "start_dist", label: "camera distance at start", unit: "yalms", decimals: 3, constant: Some("RETAIL_DIST_START") },
    Spec { id: "zoom_step", label: "zoom step per wheel notch", unit: "yalms", decimals: 3, constant: Some("ZOOM_YALMS_PER_NOTCH") },
    Spec { id: "pitch_min", label: "camera pitch minimum (down)", unit: "deg", decimals: 2, constant: Some("RETAIL_PITCH_MIN") },
    Spec { id: "pitch_max", label: "camera pitch maximum (up)", unit: "deg", decimals: 2, constant: Some("RETAIL_PITCH_MAX") },
    Spec { id: "start_pitch", label: "camera pitch at start", unit: "deg", decimals: 2, constant: Some("START_PITCH") },
    Spec { id: "orbit_key_yaw", label: "orbit rate, keys (yaw)", unit: "deg/s", decimals: 2, constant: Some("ARROW_ORBIT_DEG_PER_S") },
    Spec { id: "orbit_key_pitch", label: "orbit rate, keys (pitch)", unit: "deg/s", decimals: 2, constant: Some("ARROW_ORBIT_DEG_PER_S") },
    Spec { id: "orbit_mouse_yaw", label: "orbit rate, mouse (yaw)", unit: "deg/count", decimals: 4, constant: Some("ORBIT_DEG_PER_UNIT") },
    Spec { id: "orbit_mouse_pitch", label: "orbit rate, mouse (pitch)", unit: "deg/count", decimals: 4, constant: Some("ORBIT_DEG_PER_UNIT") },
    Spec { id: "look_height", label: "look-at height above position (py - look_y)", unit: "yalms", decimals: 3, constant: Some("RETAIL_HEAD") },
    Spec { id: "fov", label: "field of view", unit: "deg", decimals: 2, constant: Some("RETAIL_FOV_DEG") },
];

/// Metric values by `Spec::id`, plus bookkeeping about the recording.
#[derive(Clone, Debug, Default)]
pub struct Metrics {
    pub v: BTreeMap<&'static str, f64>,
    pub rows: usize,
    pub good_rows: usize,
    pub late_rows: usize,
    pub duration_s: f64,
    pub rate_hz: f64,
    /// Counts behind the harder metrics, for the notes.
    pub notes: Vec<String>,
}

impl Metrics {
    pub fn get(&self, id: &str) -> Option<f64> {
        self.v.get(id).copied()
    }
}

fn median(v: &mut Vec<f64>) -> Option<f64> {
    v.retain(|x| x.is_finite());
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    Some(if n % 2 == 1 { v[n / 2] } else { 0.5 * (v[n / 2 - 1] + v[n / 2]) })
}

fn percentile(v: &mut Vec<f64>, p: f64) -> Option<f64> {
    v.retain(|x| x.is_finite());
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let i = ((v.len() - 1) as f64 * p).round() as usize;
    Some(v[i])
}

/// Signed shortest angle from `a` to `b`, degrees.
pub fn wrap_diff(a: f64, b: f64) -> f64 {
    (b - a + 540.0).rem_euclid(360.0) - 180.0
}

fn is_move(r: &Row) -> bool {
    r.keys.w || r.keys.a || r.keys.s || r.keys.d
}

/// Maximal runs where `f` holds, as half-open index ranges.
fn runs(rows: &[Row], f: impl Fn(&Row) -> bool) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, r) in rows.iter().enumerate() {
        match (f(r), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, rows.len()));
    }
    out
}

/// Horizontal speed (x, z) from positions, central difference over +/- `k` samples.
fn speeds(rows: &[Row], k: usize) -> Vec<f64> {
    let n = rows.len();
    (0..n)
        .map(|i| {
            let (a, b) = (i.saturating_sub(k), (i + k).min(n - 1));
            let dt = rows[b].t - rows[a].t;
            if dt <= 1e-9 {
                return 0.0;
            }
            let (dx, dz) = (rows[b].pos[0] - rows[a].pos[0], rows[b].pos[2] - rows[a].pos[2]);
            (dx * dx + dz * dz).sqrt() / dt
        })
        .collect()
}

/// Rate of change of an angle series (degrees per second), central difference.
fn angle_rates(rows: &[Row], get: impl Fn(&Row) -> f64, k: usize) -> Vec<f64> {
    let n = rows.len();
    (0..n)
        .map(|i| {
            let (a, b) = (i.saturating_sub(k), (i + k).min(n - 1));
            let dt = rows[b].t - rows[a].t;
            if dt <= 1e-9 { 0.0 } else { wrap_diff(get(&rows[a]), get(&rows[b])) / dt }
        })
        .collect()
}

/// Movement metrics: steady speed, acceleration and deceleration time, follow lag.
fn movement(rows: &[Row], m: &mut Metrics) {
    let v = speeds(rows, 2);
    let (mut steady, mut accel, mut decel, mut lag_d, mut lag_t) = (vec![], vec![], vec![], vec![], vec![]);
    let mut segs = 0;
    for (i0, i1) in runs(rows, is_move) {
        let (ts, te) = (rows[i0].t, rows[i1 - 1].t);
        if te - ts < 1.5 {
            continue;
        }
        let mut s: Vec<f64> = (i0..i1).filter(|&i| rows[i].t >= ts + 1.0).map(|i| v[i]).collect();
        let Some(vss) = median(&mut s) else { continue };
        if vss < 1.0 {
            continue; // blocked or turning in place
        }
        segs += 1;
        steady.push(vss);
        if let Some(j) = (i0..rows.len()).find(|&j| v[j] >= 0.9 * vss) {
            accel.push((rows[j].t - ts).max(0.0));
        }
        let search_end = rows.iter().position(|r| r.t > te + 3.0).unwrap_or(rows.len());
        match (i1..search_end).find(|&j| v[j] <= 0.1 * vss) {
            Some(j) => decel.push((rows[j].t - te).max(0.0)),
            None if search_end > i1 && rows[search_end - 1].t >= te + 2.9 => decel.push(3.0),
            None => {}
        }
        for i in i0..i1 {
            if rows[i].t < ts + 1.0 || v[i] < 0.8 * vss {
                continue;
            }
            let (a, b) = (i.saturating_sub(4), (i + 4).min(rows.len() - 1));
            let (dx, dz) = (rows[b].pos[0] - rows[a].pos[0], rows[b].pos[2] - rows[a].pos[2]);
            let l = (dx * dx + dz * dz).sqrt();
            if l < 1e-6 {
                continue;
            }
            let (ux, uz) = (dx / l, dz / l);
            let d = (rows[i].pos[0] - rows[i].look[0]) * ux + (rows[i].pos[2] - rows[i].look[2]) * uz;
            lag_d.push(d);
            lag_t.push(d / v[i]);
        }
    }
    let mut put = |id: &'static str, x: Option<f64>| {
        if let Some(x) = x {
            m.v.insert(id, x);
        }
    };
    put("run_speed", median(&mut steady));
    put("accel_time", median(&mut accel));
    put("decel_time", median(&mut decel));
    put("follow_lag_dist", median(&mut lag_d));
    put("follow_lag_time", median(&mut lag_t));
    m.notes.push(format!("movement segments used: {segs}"));
    let mut eff: Vec<f64> = rows.iter().map(|r| r.speed_eff).collect();
    if let Some(x) = median(&mut eff) {
        m.v.insert("speed_field", x);
    }
    let mut w = angle_rates(rows, |r| r.heading_deg, 2);
    w.iter_mut().for_each(|x| *x = x.abs());
    let mut turning: Vec<f64> = w.into_iter().filter(|x| *x > 5.0).collect();
    if turning.len() >= 5 {
        if let Some(x) = percentile(&mut turning, 0.95) {
            m.v.insert("turn_rate", x);
        }
    }
}

/// Zoom step per wheel notch: from wheel events when present, else from distance plateaus.
fn zoom_step(rows: &[Row], dmin: f64, dmax: f64, m: &mut Metrics) {
    let inside = |d: f64| d > dmin + 0.03 && d < dmax - 0.03;
    let mut steps = Vec::new();
    let events: Vec<usize> = (0..rows.len()).filter(|&i| rows[i].wheel != 0).collect();
    for (k, &i) in events.iter().enumerate() {
        let next = events.get(k + 1).copied().unwrap_or(rows.len());
        let end = (i..next).take_while(|&j| rows[j].t <= rows[i].t + 1.2).last().unwrap_or(i);
        if end < i + 3 {
            continue;
        }
        let mut b: Vec<f64> = (i.saturating_sub(6)..i).map(|j| rows[j].dist).collect();
        let Some(before) = median(&mut b) else { continue };
        let tail: Vec<f64> = (end - 2..=end).map(|j| rows[j].dist).collect();
        let spread = tail.iter().cloned().fold(f64::MIN, f64::max) - tail.iter().cloned().fold(f64::MAX, f64::min);
        let after = tail[1];
        if spread > 0.02 || !inside(after) {
            continue;
        }
        let step = (before - after).abs() / rows[i].wheel.unsigned_abs() as f64;
        if step > 0.01 {
            steps.push(step);
        }
    }
    if !steps.is_empty() {
        m.notes.push(format!("zoom events used: {} of {}", steps.len(), events.len()));
        if let Some(x) = median(&mut steps) {
            m.v.insert("zoom_step", x);
        }
        return;
    }
    // No wheel data: plateaus of the distance, each at least 0.4 s long, and the jumps between them.
    let mut plateaus: Vec<f64> = Vec::new();
    for (i0, i1) in runs(rows, |_| true) {
        let mut s = i0;
        while s < i1 {
            let mut e = s;
            while e + 1 < i1 && (rows[e + 1].dist - rows[s].dist).abs() < 0.02 {
                e += 1;
            }
            if rows[e].t - rows[s].t >= 0.4 {
                let mut d: Vec<f64> = (s..=e).map(|j| rows[j].dist).collect();
                if let Some(p) = median(&mut d) {
                    plateaus.push(p);
                }
            }
            s = e + 1;
        }
    }
    let mut jumps: Vec<f64> = plateaus.windows(2).map(|w| (w[1] - w[0]).abs()).filter(|j| *j > 0.05 && *j < 3.0).collect();
    if !jumps.is_empty() {
        m.notes.push(format!("zoom step from distance plateaus ({} jumps), no wheel column data", jumps.len()));
        if let Some(x) = median(&mut jumps) {
            m.v.insert("zoom_step", x);
        }
    }
}

/// Orbit rates by key and by mouse count.
fn orbit(rows: &[Row], pmin: f64, pmax: f64, m: &mut Metrics) {
    let yaw_dir = |r: &Row| -> i32 {
        let l = r.keys.left || (r.keys.num & NUM_4 != 0);
        let rt = r.keys.right || (r.keys.num & NUM_6 != 0);
        (rt as i32) - (l as i32)
    };
    let pitch_dir = |r: &Row| -> i32 {
        let u = r.keys.up || (r.keys.num & NUM_8 != 0);
        let d = r.keys.down || (r.keys.num & NUM_2 != 0);
        (u as i32) - (d as i32)
    };
    let yaw_rate = angle_rates(rows, |r| r.cam_yaw, 2);
    let pitch_rate = angle_rates(rows, |r| r.cam_pitch, 2);
    let no_mouse = |r: &Row| !(r.keys.lmb || r.keys.rmb || r.keys.mmb);
    let mut ky = vec![];
    for (i0, i1) in runs(rows, |r| yaw_dir(r) != 0 && no_mouse(r)) {
        let t0 = rows[i0].t;
        ky.extend((i0..i1).filter(|&i| rows[i].t >= t0 + 0.3).map(|i| yaw_rate[i].abs()));
    }
    let mut kp = vec![];
    for (i0, i1) in runs(rows, |r| pitch_dir(r) != 0 && no_mouse(r)) {
        let t0 = rows[i0].t;
        kp.extend((i0..i1).filter(|&i| rows[i].t >= t0 + 0.3 && rows[i].cam_pitch > pmin + 1.0 && rows[i].cam_pitch < pmax - 1.0).map(|i| pitch_rate[i].abs()));
    }
    if let Some(x) = median(&mut ky) {
        m.v.insert("orbit_key_yaw", x);
    }
    if let Some(x) = median(&mut kp) {
        m.v.insert("orbit_key_pitch", x);
    }
    // Mouse: slope of the camera angle change against raw counts, best of 0..3 samples of delay.
    let slope = |counts: &dyn Fn(&Row) -> f64, angle: &dyn Fn(&Row) -> f64, is_yaw: bool| -> Option<f64> {
        let n = rows.len();
        let dang: Vec<f64> = (0..n).map(|i| if i == 0 { 0.0 } else { wrap_diff(angle(&rows[i - 1]), angle(&rows[i])) }).collect();
        let mut best: Option<(f64, f64)> = None;
        for lag in 0..=3usize {
            let (mut sxy, mut sxx, mut syy, mut cnt) = (0.0, 0.0, 0.0, 0);
            for i in 0..n.saturating_sub(lag) {
                let x = counts(&rows[i]);
                if x == 0.0 || !(rows[i].keys.lmb || rows[i].keys.rmb) {
                    continue;
                }
                if !is_yaw && !(rows[i + lag].cam_pitch > pmin + 0.5 && rows[i + lag].cam_pitch < pmax - 0.5) {
                    continue;
                }
                let y = dang[i + lag];
                sxy += x * y;
                sxx += x * x;
                syy += y * y;
                cnt += 1;
            }
            if cnt < 10 || sxx <= 0.0 || syy <= 0.0 {
                continue;
            }
            let corr = (sxy / (sxx * syy).sqrt()).abs();
            if best.is_none_or(|b| corr > b.0) {
                best = Some((corr, (sxy / sxx).abs()));
            }
        }
        best.filter(|b| b.0 > 0.5).map(|b| b.1)
    };
    if let Some(x) = slope(&|r| r.mouse_dx as f64, &|r| r.cam_yaw, true) {
        m.v.insert("orbit_mouse_yaw", x);
    }
    if let Some(x) = slope(&|r| r.mouse_dy as f64, &|r| r.cam_pitch, false) {
        m.v.insert("orbit_mouse_pitch", x);
    }
}

pub fn metrics(all: &[Row]) -> Metrics {
    let mut m = Metrics { rows: all.len(), ..Default::default() };
    let rows: Vec<Row> = all.iter().filter(|r| r.quality & Q_BAD_MASK == 0 && r.t.is_finite()).cloned().collect();
    m.good_rows = rows.len();
    m.late_rows = all.iter().filter(|r| r.quality & crate::rows::Q_LATE != 0).count();
    if rows.len() < 10 {
        m.notes.push("fewer than 10 good rows".into());
        return m;
    }
    m.duration_s = rows[rows.len() - 1].t - rows[0].t;
    m.rate_hz = if m.duration_s > 0.0 { (rows.len() - 1) as f64 / m.duration_s } else { 0.0 };
    movement(&rows, &mut m);
    // Range limits are the 2nd and 98th percentile of the distance, so a few frames of the camera
    // being pulled in front of a wall do not read as the zoom limit.
    let mut d: Vec<f64> = rows.iter().map(|r| r.dist).collect();
    let (dmin, dmax) = (percentile(&mut d, 0.02).unwrap_or(0.0), percentile(&mut d, 0.98).unwrap_or(0.0));
    m.v.insert("dist_min", dmin);
    m.v.insert("dist_max", dmax);
    let (pmin, pmax) = (rows.iter().map(|r| r.cam_pitch).fold(f64::MAX, f64::min), rows.iter().map(|r| r.cam_pitch).fold(f64::MIN, f64::max));
    m.v.insert("pitch_min", pmin);
    m.v.insert("pitch_max", pmax);
    let t0 = rows[0].t;
    let mut sp: Vec<f64> = rows.iter().take_while(|r| r.t < t0 + 2.0).map(|r| r.cam_pitch).collect();
    let mut sd: Vec<f64> = rows.iter().take_while(|r| r.t < t0 + 2.0).map(|r| r.dist).collect();
    if let Some(x) = median(&mut sp) {
        m.v.insert("start_pitch", x);
    }
    if let Some(x) = median(&mut sd) {
        m.v.insert("start_dist", x);
    }
    zoom_step(&rows, dmin, dmax, &mut m);
    orbit(&rows, pmin, pmax, &mut m);
    let v = speeds(&rows, 2);
    let mut h: Vec<f64> = (0..rows.len()).filter(|&i| !is_move(&rows[i]) && v[i] < 0.05).map(|i| rows[i].pos[1] - rows[i].look[1]).collect();
    if let Some(x) = median(&mut h) {
        m.v.insert("look_height", x);
    }
    let mut f: Vec<f64> = rows.iter().map(|r| r.fov_deg).collect();
    if let Some(x) = median(&mut f) {
        m.v.insert("fov", x);
    }
    m
}

fn cell(x: Option<f64>, dec: usize) -> String {
    x.map_or("n/a".into(), |v| format!("{v:.dec$}"))
}

/// The comparison table: retail value, Signet value, difference (Signet minus retail), and the
/// retail value to use for the matching Signet constant.
pub fn report(retail: &Metrics, signet: &Metrics) -> String {
    let mut o = String::new();
    let _ = writeln!(o, "Retail parity comparison (difference = Signet - retail)");
    for (n, m) in [("retail", retail), ("signet", signet)] {
        let _ = writeln!(o, "  {n}: {} rows, {} usable, {:.1} s, {:.1} Hz, {} late", m.rows, m.good_rows, m.duration_s, m.rate_hz, m.late_rows);
    }
    let _ = writeln!(o);
    let _ = writeln!(o, "{:<46} {:>10} {:>14} {:>14} {:>12}   {}", "metric", "unit", "retail", "signet", "difference", "suggested constant (from retail)");
    let _ = writeln!(o, "{}", "-".repeat(132));
    for s in SPECS {
        let (a, b) = (retail.get(s.id), signet.get(s.id));
        let diff = a.zip(b).map(|(a, b)| b - a);
        let sug = match (s.constant, a) {
            (Some(c), Some(a)) => {
                let a = if s.id == "look_height" { a.abs() } else { a };
                format!("{c} = {:.*}", s.decimals, a)
            }
            (Some(c), None) => format!("{c}: no retail value"),
            _ => String::new(),
        };
        let _ = writeln!(o, "{:<46} {:>10} {:>14} {:>14} {:>12}   {}", s.label, s.unit, cell(a, s.decimals), cell(b, s.decimals), diff.map_or("n/a".into(), |d| format!("{d:+.*}", s.decimals)), sug);
    }
    let _ = writeln!(o);
    for (n, m) in [("retail", retail), ("signet", signet)] {
        for note in &m.notes {
            let _ = writeln!(o, "note ({n}): {note}");
        }
    }
    let _ = writeln!(o, "note: pitch is the camera's view pitch, negative looks down; a retail look-at height with the opposite sign means retail y is up-positive.");
    let _ = writeln!(o, "note: BASE_RUN_SPEED is already the retail 5.0 base; a different measured speed here means speed gear or a build difference, not a constant to copy blindly.");
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::{self, Params};

    fn close(m: &Metrics, id: &str, want: f64, tol: f64) {
        let got = m.get(id).unwrap_or_else(|| panic!("{id} missing; notes {:?}", m.notes));
        assert!((got - want).abs() <= tol, "{id}: got {got}, want {want} +/- {tol}");
    }

    #[test]
    fn recovers_known_parameters_from_a_synthetic_recording() {
        let p = Params::retail_like();
        let rows = synth::generate(&p);
        let m = metrics(&rows);
        close(&m, "run_speed", p.run_speed, 0.05);
        close(&m, "accel_time", 0.9 * p.accel_time, 0.05);
        close(&m, "decel_time", 0.9 * p.decel_time, 0.06);
        close(&m, "turn_rate", p.turn_rate, 0.12 * p.turn_rate);
        close(&m, "follow_lag_time", p.follow_lag, 0.06);
        close(&m, "dist_min", p.dist_min, 0.02);
        close(&m, "dist_max", p.dist_max, 0.02);
        close(&m, "zoom_step", p.zoom_step, 0.02);
        close(&m, "pitch_min", p.pitch_min, 0.5);
        close(&m, "pitch_max", p.pitch_max, 0.5);
        close(&m, "orbit_key_yaw", p.key_yaw_rate, 3.0);
        close(&m, "orbit_key_pitch", p.key_pitch_rate, 3.0);
        close(&m, "orbit_mouse_yaw", p.mouse_deg_per_count, 0.01);
        close(&m, "orbit_mouse_pitch", p.mouse_deg_per_count, 0.01);
        close(&m, "look_height", p.look_height, 0.01);
        close(&m, "fov", 57.5, 0.01);
        close(&m, "start_pitch", p.start_pitch, 0.1);
        close(&m, "start_dist", p.dist_max, 0.05);
        assert!((m.rate_hz - 60.0).abs() < 1.0, "{}", m.rate_hz);
    }

    #[test]
    fn plateau_fallback_when_there_is_no_wheel_column() {
        let p = Params::retail_like();
        let mut rows = synth::generate(&p);
        for r in &mut rows {
            r.wheel = 0;
        }
        let m = metrics(&rows);
        close(&m, "zoom_step", p.zoom_step, 0.03);
    }

    #[test]
    fn report_has_every_metric_and_constants() {
        let p = Params::retail_like();
        let r = metrics(&synth::generate(&p));
        let q = metrics(&synth::generate(&Params::signet_like()));
        let t = report(&r, &q);
        for s in SPECS {
            assert!(t.contains(s.label), "{}", s.label);
        }
        assert!(t.contains("RETAIL_PITCH_MIN = "), "{t}");
        assert!(t.contains("ORBIT_DEG_PER_UNIT = 0.2000"), "{t}");
    }

    #[test]
    fn wrap_diff_takes_the_short_way() {
        assert!((wrap_diff(350.0, 10.0) - 20.0).abs() < 1e-9);
        assert!((wrap_diff(10.0, 350.0) + 20.0).abs() < 1e-9);
    }

    #[test]
    fn too_short_a_recording_is_reported_not_crashed() {
        let rows = synth::generate(&Params::retail_like());
        let m = metrics(&rows[..5]);
        assert!(m.get("run_speed").is_none() && !m.notes.is_empty());
    }
}
