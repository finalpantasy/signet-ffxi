//! Collision gap tools.
//!   signet-dat-gaps cells <zone .DAT> [root]   how far each placed mesh's bounds reach past the cells listing it
//!   signet-dat-gaps line <zone .DAT> x0 z0 x1 z1 [n] [root]   sample ground height along a line
//!   signet-dat-gaps scan [zones] [lines] [root]   gap rate over many zones (random lines)
use signet_dat::collision::{CELL, Collision};
use signet_dat::crypt::{self, ZoneKeys};
use signet_dat::{Install, chunks, find_install_root};
use std::collections::HashSet;
use std::path::PathBuf;

fn mzb_of(data: &[u8], keys: &ZoneKeys) -> Option<Vec<u8>> {
    for c in chunks(data).filter_map(Result::ok) {
        if c.kind == crypt::MZB_KIND {
            let mut p = c.data.to_vec();
            crypt::decode_mzb(&mut p, keys);
            return Some(p);
        }
    }
    None
}

fn open(path: &str, root: &std::path::Path) -> Collision {
    let keys = ZoneKeys::from_install(root).unwrap();
    Collision::parse(&mzb_of(&std::fs::read(path).unwrap(), &keys).unwrap()).unwrap()
}

fn cells(col: &Collision) {
    let mut lists: Vec<Vec<usize>> = vec![Vec::new(); col.placed.len()];
    for (c, items) in col.non_empty_cells() {
        for &pi in items {
            lists[pi as usize].push(c);
        }
    }
    let (mut pairs, mut missing, mut extra) = (0, 0, 0);
    let mut depth_hist = [0u32; 12];
    let mut worst = 0.0f32;
    for (pi, l) in lists.iter().enumerate() {
        let mut lo = [f32::MAX; 2];
        let mut hi = [f32::MIN; 2];
        for ti in 0..col.meshes[col.placed[pi].mesh as usize].tris.len() {
            for q in col.world_tri(pi as u32, ti as u32) {
                for (k, a) in [0usize, 2].into_iter().enumerate() {
                    lo[k] = lo[k].min(q[a]);
                    hi[k] = hi[k].max(q[a]);
                }
            }
        }
        if lo[0] > hi[0] {
            continue;
        }
        pairs += 1;
        let set: HashSet<usize> = l.iter().copied().collect();
        let r = |v: f32, k: usize| ((v - col.origin[k]) / CELL).floor() as i64;
        let (x0, x1, z0, z1) = (r(lo[0], 0), r(hi[0], 0), r(lo[1], 1), r(hi[1], 1));
        for cz in z0.max(0)..=z1.min(col.rows as i64 - 1) {
            for cx in x0.max(0)..=x1.min(col.cols as i64 - 1) {
                if !set.contains(&(cz as usize * col.cols + cx as usize)) {
                    missing += 1;
                    let ox = (hi[0].min(col.origin[0] + (cx + 1) as f32 * CELL) - lo[0].max(col.origin[0] + cx as f32 * CELL)).max(0.0);
                    let oz = (hi[1].min(col.origin[1] + (cz + 1) as f32 * CELL) - lo[1].max(col.origin[1] + cz as f32 * CELL)).max(0.0);
                    let d = ox.min(oz);
                    worst = worst.max(d);
                    depth_hist[((d / 0.01) as usize).min(11)] += 1;
                }
            }
        }
        for &c in l {
            let (cx, cz) = ((c % col.cols) as i64, (c / col.cols) as i64);
            if cx < x0 || cx > x1 || cz < z0 || cz > z1 {
                extra += 1;
            }
        }
    }
    println!("pairs {pairs}; bbox cells not listed {missing}; listed cells outside bbox {extra}; worst missing overlap {worst:.3}");
    println!("missing overlap depth histogram (0.01 buckets, last = >=0.11): {depth_hist:?}");
}


fn probe(col: &Collision, x: f32, z: f32) {
    let mut lists: Vec<Vec<usize>> = vec![Vec::new(); col.placed.len()];
    for (c, items) in col.non_empty_cells() {
        for &pi in items {
            lists[pi as usize].push(c);
        }
    }
    println!("cell of point {:?}", col.cell_of(x, z));
    col.for_each_triangle(|pi, ti, w, _| {
        let sign = |a: [f32; 3], b: [f32; 3]| (b[0] - a[0]) * (z - a[2]) - (b[2] - a[2]) * (x - a[0]);
        let (d0, d1, d2) = (sign(w[0], w[1]), sign(w[1], w[2]), sign(w[2], w[0]));
        if (d0 >= 0.0 && d1 >= 0.0 && d2 >= 0.0) || (d0 <= 0.0 && d1 <= 0.0 && d2 <= 0.0) {
            let cs: Vec<(usize, usize)> = lists[pi as usize].iter().map(|c| (c % col.cols, c / col.cols)).collect();
            println!("pair {pi} tri {ti} corners {w:?}
   listed in cells {cs:?}");
        }
    });
}

/// 2D triangle vs rectangle overlap (separating axis), rectangle given as lo/hi.
fn tri_rect(w: &[[f32; 3]; 3], lo: [f32; 2], hi: [f32; 2]) -> bool {
    let p: Vec<[f32; 2]> = w.iter().map(|q| [q[0], q[2]]).collect();
    for k in 0..2 {
        if p.iter().all(|q| q[k] < lo[k]) || p.iter().all(|q| q[k] > hi[k]) {
            return false;
        }
    }
    let r = [[lo[0], lo[1]], [hi[0], lo[1]], [hi[0], hi[1]], [lo[0], hi[1]]];
    for i in 0..3 {
        let (a, b) = (p[i], p[(i + 1) % 3]);
        let n = [b[1] - a[1], -(b[0] - a[0])];
        let d = |q: [f32; 2]| (q[0] - a[0]) * n[0] + (q[1] - a[1]) * n[1];
        let tri_max = p.iter().map(|&q| d(q)).fold(f32::MIN, f32::max);
        let tri_min = p.iter().map(|&q| d(q)).fold(f32::MAX, f32::min);
        let (rmin, rmax) = (r.iter().map(|&q| d(q)).fold(f32::MAX, f32::min), r.iter().map(|&q| d(q)).fold(f32::MIN, f32::max));
        if rmin > tri_max || rmax < tri_min {
            return false;
        }
    }
    true
}

/// For every triangle and every cell it truly overlaps where its mesh is not listed, how deep
/// (largest shrink of the cell that still overlaps).
fn depth(col: &Collision) {
    let mut lists: Vec<HashSet<usize>> = vec![HashSet::new(); col.placed.len()];
    for (c, items) in col.non_empty_cells() {
        for &pi in items {
            lists[pi as usize].insert(c);
        }
    }
    let shr = [0.0f32, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0];
    let mut hist = vec![0u64; shr.len()];
    let (mut tris, mut bad_tris) = (0u64, 0u64);
    col.for_each_triangle(|pi, _ti, w, _| {
        tris += 1;
        let lo = |k: usize| w.iter().map(|q| q[k]).fold(f32::MAX, f32::min);
        let hi = |k: usize| w.iter().map(|q| q[k]).fold(f32::MIN, f32::max);
        let r = |v: f32, k: usize| ((v - col.origin[k]) / CELL).floor() as i64;
        let mut bad = false;
        for cz in r(lo(2), 1).max(0)..=r(hi(2), 1).min(col.rows as i64 - 1) {
            for cx in r(lo(0), 0).max(0)..=r(hi(0), 0).min(col.cols as i64 - 1) {
                if lists[pi as usize].contains(&(cz as usize * col.cols + cx as usize)) {
                    continue;
                }
                let (x0, z0) = (col.origin[0] + cx as f32 * CELL, col.origin[1] + cz as f32 * CELL);
                let mut d = None;
                for (i, &e) in shr.iter().enumerate() {
                    if tri_rect(&w, [x0 + e, z0 + e], [x0 + CELL - e, z0 + CELL - e]) {
                        d = Some(i);
                    }
                }
                if let Some(i) = d {
                    hist[i] += 1;
                    bad = true;
                }
            }
        }
        if bad {
            bad_tris += 1;
        }
    });
    println!("triangles {tris}; with an unlisted overlapped cell {bad_tris}");
    println!("unlisted overlaps by max-shrink bucket {shr:?} (counts, bucket i = overlaps shrunk cell by shr[i] but not shr[i+1]): {hist:?}");
}

fn single(col: &Collision) {
    let mut lists: Vec<Vec<usize>> = vec![Vec::new(); col.placed.len()];
    for (c, items) in col.non_empty_cells() {
        for &pi in items {
            lists[pi as usize].push(c);
        }
    }
    let mut multi = 0;
    let mut cnt = std::collections::BTreeMap::new();
    let (mut cl, mut ch, mut cc) = (0, 0, 0); // bbox min in cell, max in cell, center in cell
    let mut n = 0;
    let mut dump = 0;
    for (pi, l) in lists.iter().enumerate() {
        *cnt.entry(l.len()).or_insert(0) += 1;
        if l.len() != 1 {
            multi += 1;
            continue;
        }
        let mut lo = [f32::MAX; 2];
        let mut hi = [f32::MIN; 2];
        for ti in 0..col.meshes[col.placed[pi].mesh as usize].tris.len() {
            for q in col.world_tri(pi as u32, ti as u32) {
                for (k, a) in [0usize, 2].into_iter().enumerate() {
                    lo[k] = lo[k].min(q[a]);
                    hi[k] = hi[k].max(q[a]);
                }
            }
        }
        let c = l[0];
        let (c0, c1) = ([col.origin[0] + (c % col.cols) as f32 * CELL, col.origin[1] + (c / col.cols) as f32 * CELL], [col.origin[0] + (c % col.cols + 1) as f32 * CELL, col.origin[1] + (c / col.cols + 1) as f32 * CELL]);
        let inn = |v: [f32; 2]| v[0] >= c0[0] && v[0] < c1[0] && v[1] >= c0[1] && v[1] < c1[1];
        n += 1;
        cl += inn(lo) as i32;
        ch += inn(hi) as i32;
        cc += inn([(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0]) as i32;
        if dump < 8 && !inn([(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0]) {
            dump += 1;
            println!("cell lo {c0:?} hi {c1:?} bbox lo {lo:?} hi {hi:?}");
        }
    }
    println!("listing-count histogram {cnt:?}; single-listed {n}: bbox min in cell {cl}, bbox max in cell {ch}, center in cell {cc}");
}

fn line(col: &Collision, a: [f32; 2], b: [f32; 2], n: usize) {
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let (x, z) = (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
        println!("{x:.3} {z:.3} {:?}", col.ground_height(x, z));
    }
}

/// Random lines from non-empty cells, sampled every 0.01 yalm. A gap sample is a no-surface
/// sample in a run of at most 15 (0.15 yalm) with a surface on both sides. Returns
/// (samples between the first and last surface on each line, gap samples).
fn gaps(col: &Collision, lines: usize, seed: &mut u64, old: bool) -> (u64, u64) {
    let mut rnd = || {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (*seed >> 33) as f32 / (1u64 << 31) as f32
    };
    let ne: Vec<usize> = col.non_empty_cells().map(|(c, _)| c).collect();
    if ne.is_empty() {
        return (0, 0);
    }
    let (mut n, mut g) = (0u64, 0u64);
    for _ in 0..lines {
        let c = ne[(rnd() * ne.len() as f32) as usize % ne.len()];
        let x0 = col.origin[0] + ((c % col.cols) as f32 + rnd()) * CELL;
        let z0 = col.origin[1] + ((c / col.cols) as f32 + rnd()) * CELL;
        let ang = rnd() * std::f32::consts::TAU;
        let (dx, dz) = (ang.cos(), ang.sin());
        let mut run = 0u64;
        let mut seen_hit = false;
        for i in 0..2000 {
            let (x, z) = (x0 + dx * 0.01 * i as f32, z0 + dz * 0.01 * i as f32);
            if (if old { col.ground_height_listed_only(x, z) } else { col.ground_height(x, z) }).is_some() {
                if seen_hit {
                    n += run + 1;
                    if run <= 15 {
                        g += run;
                    }
                }
                seen_hit = true;
                run = 0;
            } else {
                run += 1;
            }
        }
    }
    (n, g)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root_of = |i: usize| args.get(i).map(PathBuf::from).or_else(find_install_root).expect("no install root");
    match args.first().map(String::as_str) {
        Some("cells") if args.len() >= 2 => cells(&open(&args[1], &root_of(2))),
        Some("probe") if args.len() >= 4 => probe(&open(&args[1], &root_of(4)), args[2].parse().unwrap(), args[3].parse().unwrap()),
        Some("depth") if args.len() >= 2 => depth(&open(&args[1], &root_of(2))),
        Some("single") if args.len() >= 2 => single(&open(&args[1], &root_of(2))),
        Some("line") if args.len() >= 6 => {
            let f = |i: usize| args[i].parse::<f32>().unwrap();
            let n = args.get(6).and_then(|s| s.parse().ok()).unwrap_or(100);
            line(&open(&args[1], &root_of(7)), [f(2), f(3)], [f(4), f(5)], n)
        }
        Some("scan") => {
            let nz: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(30);
            let nl: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(300);
            let root = root_of(3);
            let keys = ZoneKeys::from_install(&root).unwrap();
            let install = Install::open(&root).unwrap();
            let mut seen = HashSet::new();
            let (mut tn, mut tg, mut tn0, mut tg0, mut zones) = (0u64, 0u64, 0u64, 0u64, 0);
            let mut seed = 99u64;
            for id in 0..install.id_count() {
                let Some(loc) = install.locate(id) else { continue };
                if !seen.insert(loc) {
                    continue;
                }
                let Ok(data) = std::fs::read(install.path(loc)) else { continue };
                if data.len() < 100_000 {
                    continue;
                }
                let Some(p) = mzb_of(&data, &keys) else { continue };
                let Ok(col) = Collision::parse(&p) else { continue };
                if col.non_empty_cells().count() < 50 {
                    continue;
                }
                let mut s2 = seed;
                let (n, g) = gaps(&col, nl, &mut seed, false);
                let (n0, g0) = gaps(&col, nl, &mut s2, true);
                println!("{:?}: before samples {n0} gap {g0}; after samples {n} gap {g}", loc.relative_path());
                tn += n;
                tg += g;
                tn0 += n0;
                tg0 += g0;
                zones += 1;
                if zones >= nz {
                    break;
                }
            }
            println!("TOTAL zones {zones}: before {tn0} samples, {tg0} gap, rate {:.4}%; after {tn} samples, {tg} gap, rate {:.4}%", 100.0 * tg0 as f64 / tn0.max(1) as f64, 100.0 * tg as f64 / tn.max(1) as f64);
        }
        _ => eprintln!("usage: signet-dat-gaps cells|line|scan"),
    }
}
