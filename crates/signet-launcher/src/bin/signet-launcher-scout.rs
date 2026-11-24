//! Finds open vistas in a zone's collision mesh for launcher hero art.
//! Usage: signet-launcher-scout <zone.DAT> [--root <install>] [--top N] [--eye H] [--step S]
//! Prints `x y z yaw pitch score` camera lines for `signet-render --camera`, best first.
//! Score: average clear distance of a fan of rays from eye height, so spots that look out over
//! open ground win. Collision only (trees without collision are invisible to it).

use std::path::PathBuf;

use signet_dat::collision::Collision;
use signet_dat::crypt::{self, ZoneKeys};
use signet_dat::{chunks, find_install_root};

fn main() {
    if let Err(e) = run() {
        eprintln!("scout: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let dat: PathBuf = args.next().ok_or("usage: signet-launcher-scout <zone.DAT> [--root R] [--top N] [--eye H] [--step S]")?.into();
    let (mut root, mut top, mut eye, mut step) = (None, 12usize, 8.0f32, 0.0f32);
    while let Some(a) = args.next() {
        let v = args.next().ok_or(format!("{a} needs a value"))?;
        match a.as_str() {
            "--root" => root = Some(PathBuf::from(v)),
            "--top" => top = v.parse().map_err(|_| "bad --top")?,
            "--eye" => eye = v.parse().map_err(|_| "bad --eye")?,
            "--step" => step = v.parse().map_err(|_| "bad --step")?,
            _ => return Err(format!("unknown option {a}")),
        }
    }
    let root = root.or_else(find_install_root).ok_or("no install root")?;
    let bytes = std::fs::read(&dat).map_err(|e| format!("read {}: {e}", dat.display()))?;
    let keys = ZoneKeys::from_install(&root).map_err(|e| format!("keys: {e:?}"))?;
    let mut col = None;
    for c in chunks(&bytes).filter_map(Result::ok) {
        if c.kind == crypt::MZB_KIND {
            let mut p = c.data.to_vec();
            crypt::decode_mzb(&mut p, &keys);
            col = Collision::parse(&p).ok();
        }
    }
    let col = col.ok_or("no decodable collision")?;
    let (w, h) = (col.cols as f32 * signet_dat::collision::CELL, col.rows as f32 * signet_dat::collision::CELL);
    let step = if step > 0.0 { step } else { (w.max(h) / 60.0).max(4.0) };
    println!("# grid origin {:?} size {w:.0}x{h:.0} step {step:.1}", col.origin);
    let mut found: Vec<(f32, [f32; 5])> = Vec::new();
    let mut z = col.origin[1] + step / 2.0;
    while z < col.origin[1] + h {
        let mut x = col.origin[0] + step / 2.0;
        while x < col.origin[0] + w {
            let edge = x < col.origin[0] + step * 3.0 || z < col.origin[1] + step * 3.0 || x > col.origin[0] + w - step * 3.0 || z > col.origin[1] + h - step * 3.0;
            if let Some(g) = col.ground_height(x, z).filter(|_| !edge) {
                let e = [x, g - eye, z];
                // Skip spots with a ceiling right above (inside walls or under bridges).
                let roofed = col.raycast(e, [0.0, -1.0, 0.0], 6.0).is_some();
                if !roofed {
                    for k in 0..16 {
                        let yaw = k as f32 * 22.5;
                        let ray = |dy: f32, pitch: f32, max: f32| {
                            let (y, p) = ((yaw + dy).to_radians(), pitch.to_radians());
                            col.raycast(e, [p.cos() * y.sin(), -p.sin(), p.cos() * y.cos()], max).map(|h| h.t)
                        };
                        // Long sight lines down onto ground; rays into the void (off the map) cost.
                        let mut score = 0.0;
                        for dy in [-20.0f32, -10.0, 0.0, 10.0, 20.0] {
                            for pitch in [-3.0f32, -6.0, -10.0] {
                                score += ray(dy, pitch, 300.0).map_or(-100.0, |t| t.min(300.0));
                            }
                            // A wall right in front is a bad frame.
                            if ray(dy, 2.0, 40.0).is_some() {
                                score -= 150.0;
                            }
                        }
                        found.push((score / 15.0, [x, e[1], z, yaw, -6.0]));
                    }
                }
            }
            x += step;
        }
        z += step;
    }
    found.sort_by(|a, b| b.0.total_cmp(&a.0));
    // Keep spots at least 4 steps apart so the list is varied.
    let mut picked: Vec<(f32, [f32; 5])> = Vec::new();
    for f in found {
        if picked.iter().all(|p| ((p.1[0] - f.1[0]).powi(2) + (p.1[2] - f.1[2]).powi(2)).sqrt() > step * 4.0) {
            picked.push(f);
        }
        if picked.len() >= top {
            break;
        }
    }
    for (s, c) in picked {
        println!("{:.1},{:.1},{:.1},{:.0},{:.0} score {s:.0}", c[0], c[1], c[2], c[3], c[4]);
    }
    Ok(())
}
