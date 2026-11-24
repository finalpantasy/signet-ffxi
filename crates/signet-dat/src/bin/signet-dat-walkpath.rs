//! Finds a walkable route between two points of a zone with a grid search over the collision
//! data (ground continuity plus a knee-high wall check), and prints waypoints with the heading of
//! each leg. A helper for planning scripted walks.
//! Usage: signet-dat-walkpath <zone-id> <x> <z> <goal-x> <goal-z> [--root <install>]
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use signet_dat::collision::Collision;
use signet_dat::crypt::{self, ZoneKeys};
use signet_dat::zones::model_file_id;
use signet_dat::{Install, chunks, find_install_root};

const STEP: f32 = 2.0;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let root = a.iter().position(|x| x == "--root").and_then(|i| a.get(i + 1)).map(Into::into).or_else(find_install_root).expect("no install");
    let n = |i: usize| a[i].parse::<f32>().expect("number");
    let zone: u16 = a[0].parse().expect("zone id");
    let (sx, sz, gx, gz) = (n(1), n(2), n(3), n(4));
    let install = Install::open(&root).expect("open");
    let dat = install.read(model_file_id(zone)).expect("zone dat");
    let keys = ZoneKeys::from_install(&root).expect("keys");
    let mut col: Option<Collision> = None;
    for c in chunks(&dat).filter_map(Result::ok) {
        if c.kind == crypt::MZB_KIND && col.is_none() {
            let mut p = c.data.to_vec();
            crypt::decode_mzb(&mut p, &keys);
            col = Collision::parse(&p).ok();
        }
    }
    let col = col.expect("no collision");
    let y0 = col.ground_height(sx, sz).expect("no ground at start");
    let key = |x: f32, z: f32| ((x / STEP).round() as i32, (z / STEP).round() as i32);
    let pos = |k: (i32, i32)| (k.0 as f32 * STEP, k.1 as f32 * STEP);
    let start = key(sx, sz);
    let goal = key(gx, gz);
    let mut ground: HashMap<(i32, i32), f32> = HashMap::new();
    ground.insert(start, y0);
    let mut best: HashMap<(i32, i32), f32> = HashMap::new();
    let mut from: HashMap<(i32, i32), (i32, i32)> = HashMap::new();
    let mut heap = BinaryHeap::new();
    let h = |k: (i32, i32)| {
        let (x, z) = pos(k);
        ((x - gx).powi(2) + (z - gz).powi(2)).sqrt()
    };
    best.insert(start, 0.0);
    heap.push((Reverse((h(start) * 1000.0) as i64), start));
    let mut expanded = 0;
    while let Some((_, k)) = heap.pop() {
        expanded += 1;
        if (h(k) < 6.0) || expanded > 400_000 {
            // done
            let mut path = vec![k];
            let mut c = k;
            while let Some(&p) = from.get(&c) {
                path.push(p);
                c = p;
            }
            path.reverse();
            println!("route found after {expanded} expansions, {} grid steps, goal distance {:.1}", path.len(), h(k));
            let mut last = path[0];
            let mut leg_start = path[0];
            let mut leg_yaw = f32::NAN;
            for (i, &p) in path.iter().enumerate().skip(1) {
                let (x0, z0) = pos(last);
                let (x1, z1) = pos(p);
                let yaw = (x1 - x0).atan2(z1 - z0).to_degrees().rem_euclid(360.0);
                if leg_yaw.is_nan() {
                    leg_yaw = yaw;
                }
                let turned = (yaw - leg_yaw + 540.0).rem_euclid(360.0) - 180.0;
                if turned.abs() > 25.0 || i == path.len() - 1 {
                    let (lx, lz) = pos(leg_start);
                    let (ex, ez) = pos(p);
                    println!("leg: from ({lx:.0},{lz:.0}) to ({ex:.0},{ez:.0}) yaw {leg_yaw:.0} length {:.1} yalms", ((ex - lx).powi(2) + (ez - lz).powi(2)).sqrt());
                    leg_start = p;
                    leg_yaw = f32::NAN;
                }
                last = p;
            }
            return;
        }
        let (x, z) = pos(k);
        let y = ground[&k];
        let g = best[&k];
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
            let nk = (k.0 + dx, k.1 + dz);
            let (nx, nz) = pos(nk);
            let Some(ny) = col.ground_height_below(nx, y - 1.0, nz) else { continue };
            if (ny - y).abs() > 1.5 {
                continue;
            }
            // wall check at knee height along the step
            let (ddx, ddz) = (nx - x, nz - z);
            let len = (ddx * ddx + ddz * ddz).sqrt();
            let o = [x, y - 0.6, z];
            if col.raycast(o, [ddx / len, 0.0, ddz / len], len + 0.5).is_some() {
                continue;
            }
            let ng = g + len;
            if best.get(&nk).is_none_or(|&b| ng < b) {
                best.insert(nk, ng);
                ground.insert(nk, ny);
                from.insert(nk, k);
                heap.push((Reverse(((ng + h(nk)) * 1000.0) as i64), nk));
            }
        }
    }
    println!("no route");
}
