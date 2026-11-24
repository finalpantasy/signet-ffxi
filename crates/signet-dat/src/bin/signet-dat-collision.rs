//! Zone collision tools.
//!   signet-dat-collision census [install-root]
//!   signet-dat-collision shot <zone .DAT> <out-prefix> [size] [install-root]
//!       writes <prefix>_collision.png (slope-shaded) and <prefix>_visual.png (flat visual
//!       geometry, same framing as signet-dat-zoneshot)
//!   signet-dat-collision heights <zone .DAT> [install-root]
//!       compares collision ground height with the visual mesh at sample points
//!   signet-dat-collision info <zone .DAT> [install-root]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use signet_dat::collision::{CELL, Collision};
use signet_dat::crypt::{self, ZoneKeys};
use signet_dat::{Install, chunks, find_install_root, mmb, png};

type V3 = [f32; 3];

fn rotate(v: V3, r: V3) -> V3 {
    let (sx, cx) = r[0].sin_cos();
    let (sy, cy) = r[1].sin_cos();
    let (sz, cz) = r[2].sin_cos();
    let v = [v[0], v[1] * cx - v[2] * sx, v[1] * sx + v[2] * cx];
    let v = [v[0] * cy + v[2] * sy, v[1], -v[0] * sy + v[2] * cy];
    [v[0] * cz - v[1] * sz, v[0] * sz + v[1] * cz, v[2]]
}

/// Decoded pieces of one zone DAT.
struct Zone {
    mzb: Vec<u8>,
    meshes: HashMap<String, mmb::Mesh>,
}

fn load_zone(data: &[u8], keys: &ZoneKeys) -> Option<Zone> {
    let mut mzb = None;
    let mut meshes = HashMap::new();
    for c in chunks(data).filter_map(Result::ok) {
        match c.kind {
            crypt::MZB_KIND if mzb.is_none() => {
                let mut p = c.data.to_vec();
                crypt::decode_mzb(&mut p, keys);
                mzb = Some(p);
            }
            crypt::MMB_KIND => {
                let mut p = c.data.to_vec();
                crypt::decode_mmb(&mut p, keys);
                if let Ok(m) = mmb::parse(&p) {
                    meshes.insert(m.name.clone(), m);
                }
            }
            _ => {}
        }
    }
    Some(Zone { mzb: mzb?, meshes })
}

/// World-space visual triangles (corners, normal y) of every placement.
fn visual_tris(z: &Zone) -> Vec<([V3; 3], f32)> {
    let mut out = Vec::new();
    for pl in crypt::placements(&z.mzb) {
        let Some(m) = z.meshes.get(&pl.mesh) else { continue };
        for s in &m.submeshes {
            let world: Vec<V3> = s
                .vertices
                .iter()
                .map(|v| {
                    let sc = [v.position[0] * pl.scale[0], v.position[1] * pl.scale[1], v.position[2] * pl.scale[2]];
                    let r = rotate(sc, pl.rotation);
                    [r[0] + pl.position[0], r[1] + pl.position[1], r[2] + pl.position[2]]
                })
                .collect();
            for t in s.indices.chunks_exact(3) {
                let idx = [t[0] as usize, t[1] as usize, t[2] as usize];
                if idx.iter().any(|&i| i >= world.len()) {
                    continue;
                }
                let n = rotate(s.vertices[idx[0]].normal, pl.rotation);
                out.push((idx.map(|i| world[i]), n[1].abs().min(1.0)));
            }
        }
    }
    out
}

fn normal_of(w: &[V3; 3]) -> V3 {
    let (a, b) = ([w[1][0] - w[0][0], w[1][1] - w[0][1], w[1][2] - w[0][2]], [w[2][0] - w[0][0], w[2][1] - w[0][1], w[2][2] - w[0][2]]);
    let n = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if l > 0.0 { [n[0] / l, n[1] / l, n[2] / l] } else { [0.0, 1.0, 0.0] }
}

/// Top-down depth-buffered raster; x right, z down the image, smaller y wins (higher).
fn raster(tris: &[([V3; 3], [u8; 3])], lo: V3, span: f32, size: usize) -> Vec<u8> {
    let scale = (size - 1) as f32 / span;
    let mut color = vec![[24u8, 24, 32, 255]; size * size];
    let mut depth = vec![f32::MAX; size * size];
    for (p, rgb) in tris {
        let s = p.map(|q| [(q[0] - lo[0]) * scale, (q[2] - lo[2]) * scale, q[1]]);
        let area = (s[1][0] - s[0][0]) * (s[2][1] - s[0][1]) - (s[2][0] - s[0][0]) * (s[1][1] - s[0][1]);
        if area.abs() < 1e-6 {
            continue;
        }
        let x0 = s.iter().map(|q| q[0]).fold(f32::MAX, f32::min).max(0.0) as usize;
        let x1 = (s.iter().map(|q| q[0]).fold(f32::MIN, f32::max).max(0.0) as usize).min(size - 1);
        let y0 = s.iter().map(|q| q[1]).fold(f32::MAX, f32::min).max(0.0) as usize;
        let y1 = (s.iter().map(|q| q[1]).fold(f32::MIN, f32::max).max(0.0) as usize).min(size - 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = ((s[1][0] - px) * (s[2][1] - py) - (s[2][0] - px) * (s[1][1] - py)) / area;
                let w1 = ((s[2][0] - px) * (s[0][1] - py) - (s[0][0] - px) * (s[2][1] - py)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let z = w0 * s[0][2] + w1 * s[1][2] + w2 * s[2][2];
                let i = y * size + x;
                if z < depth[i] {
                    depth[i] = z;
                    color[i] = [rgb[0], rgb[1], rgb[2], 255];
                }
            }
        }
    }
    color.into_iter().flatten().collect()
}

/// Slope color from the unit normal's |y|: flat green, walkable yellow, steep orange, wall red.
fn slope_color(ny: f32) -> [u8; 3] {
    let a = ny.abs();
    let (base, k) = if a > 0.95 {
        ([60.0, 200.0, 80.0], a)
    } else if a > 0.7 {
        ([220.0, 210.0, 60.0], a)
    } else if a > 0.4 {
        ([240.0, 140.0, 40.0], 0.9)
    } else {
        ([230.0, 50.0, 50.0], 0.9)
    };
    base.map(|c| (c * (0.6 + 0.4 * k)) as u8)
}

fn hist(map: &mut BTreeMap<String, u64>, k: impl Into<String>) {
    *map.entry(k.into()).or_default() += 1;
}

fn census(root: &std::path::Path, keys: &ZoneKeys) {
    let install = Install::open(root).expect("open install");
    let mut seen = HashSet::new();
    let mut results: BTreeMap<String, u64> = BTreeMap::new();
    let (mut zones_ok, mut meshes, mut tris, mut placed, mut cells, mut pairs) = (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    let (mut nonfinite, mut outside_grid, mut cell_miss, mut cell_pairs, mut norm_bad, mut norm_tot) = (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    let mut flags: BTreeMap<String, u64> = BTreeMap::new();
    let mut slopes: BTreeMap<String, u64> = BTreeMap::new();
    let mut dims: BTreeMap<String, u64> = BTreeMap::new();
    let mut extras: BTreeMap<String, u64> = BTreeMap::new();
    let mut nflags: BTreeMap<String, u64> = BTreeMap::new();
    let mut examples: Vec<String> = Vec::new();
    let mut bad_grid: Vec<String> = Vec::new();
    let label_of = |v: &Vec<String>| v.last().cloned().unwrap_or_default();
    let mut with_zone: Vec<String> = Vec::new();
    for id in 0..install.id_count() {
        let Some(loc) = install.locate(id) else { continue };
        if !seen.insert(loc) {
            continue;
        }
        let Ok(data) = std::fs::read(install.path(loc)) else { continue };
        for c in chunks(&data).filter_map(Result::ok).filter(|c| c.kind == crypt::MZB_KIND) {
            let mut p = c.data.to_vec();
            let label = format!("{:?}", loc.relative_path());
            let d = crypt::decode_mzb(&mut p, keys);
            if d == crypt::Decoded::BadLength {
                hist(&mut results, "mzb decode: bad length");
                continue;
            }
            match Collision::parse(&p) {
                Err(e) => {
                    let why = match &e {
                        signet_dat::collision::CollisionError::NoCollision => "no collision block".to_string(),
                        signet_dat::collision::CollisionError::BadHeader(w) => format!("bad header: {w}"),
                        signet_dat::collision::CollisionError::BadCell { why, .. } => format!("bad cell: {why}"),
                        signet_dat::collision::CollisionError::BadMesh { why, .. } => format!("bad mesh: {why}"),
                    };
                    if examples.iter().filter(|x| x.contains(&why)).count() < 3 {
                        examples.push(format!("{label}: {e} (placements {})", crypt::placements(&p).len()));
                    }
                    hist(&mut results, why.clone());
                }
                Ok(col) => {
                    hist(&mut results, "ok");
                    zones_ok += 1;
                    with_zone.push(label);
                    hist(&mut dims, format!("{}x{}", col.cols, col.rows));
                    meshes += col.meshes.len() as u64;
                    placed += col.placed.len() as u64;
                    tris += col.triangle_count() as u64;
                    let nc = col.non_empty_cells().count() as u64;
                    cells += nc;
                    let span = [col.cols as f32 * CELL, col.rows as f32 * CELL];
                    for m in &col.meshes {
                        hist(&mut extras, m.extra.to_string());
                        for t in &m.tris {
                            hist(&mut nflags, t.normal_flags.to_string());
                            hist(&mut flags, format!("{}{}{}", t.flags[0], t.flags[1], t.flags[2]));
                        }
                        for n in &m.normals {
                            norm_tot += 1;
                            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                            if !l.is_finite() || (l - 1.0).abs() > 0.01 {
                                norm_bad += 1;
                            }
                        }
                    }
                    col.for_each_triangle(|pi, _ti, w, _f| {
                        if w.iter().flatten().any(|v| !v.is_finite()) {
                            nonfinite += 1;
                            return;
                        }
                        if w.iter().any(|q| q[0] < col.origin[0] || q[0] > col.origin[0] + span[0] || q[2] < col.origin[1] || q[2] > col.origin[1] + span[1]) {
                            outside_grid += 1;
                        }
                        let n = normal_of(&w);
                        let _ = pi;
                        let a = n[1].abs();
                        hist(&mut slopes, if a > 0.95 { "flat >0.95" } else if a > 0.7 { "walkable 0.7-0.95" } else if a > 0.4 { "steep 0.4-0.7" } else { "wall <0.4" });
                    });
                    let (miss0, pairs0) = (cell_miss, cell_pairs);
                    // grid consistency: every listed pair has a triangle touching its cell
                    for (c, items) in col.non_empty_cells() {
                        let (cx, cz) = ((c % col.cols) as f32, (c / col.cols) as f32);
                        let (x0, z0) = (col.origin[0] + cx * CELL - 0.05, col.origin[1] + cz * CELL - 0.05);
                        for &pi in items {
                            pairs += 1;
                            cell_pairs += 1;
                            let m = &col.meshes[col.placed[pi as usize].mesh as usize];
                            let hit = (0..m.tris.len()).any(|ti| {
                                let w = col.world_tri(pi, ti as u32);
                                let (mn, mx) = (
                                    [0, 2].map(|k| w.iter().map(|q| q[k]).fold(f32::MAX, f32::min)),
                                    [0, 2].map(|k| w.iter().map(|q| q[k]).fold(f32::MIN, f32::max)),
                                );
                                mx[0] >= x0 && mn[0] <= x0 + CELL + 0.1 && mx[1] >= z0 && mn[1] <= z0 + CELL + 0.1
                            });
                            if !hit {
                                cell_miss += 1;
                            }
                        }
                    }
                    if (cell_miss - miss0) * 5 > (cell_pairs - pairs0) {
                        bad_grid.push(format!("{} {}/{}", label_of(&with_zone), cell_miss - miss0, cell_pairs - pairs0));
                    }
                }
            }
        }
    }
    println!("zones with >20% of cell pairs missing: {bad_grid:?}");
    println!("zone MZB chunks by result:");
    for (k, v) in &results {
        println!("  {v:>5}  {k}");
    }
    for e in &examples {
        println!("  e.g. {e}");
    }
    println!("ok zones {zones_ok}; grid sizes {dims:?}");
    println!("totals over ok zones: distinct meshes {meshes}, placed (instance,mesh) pairs {placed}, triangles {tris}, non-empty cells {cells}, cell-list pairs {pairs}");
    println!("sanity: non-finite tris {nonfinite}, tris outside grid square {outside_grid}, normals not unit {norm_bad}/{norm_tot}, cell pairs with no triangle in cell {cell_miss}/{cell_pairs}");
    println!("corner-flag patterns (bit k = corner k flagged): {flags:?}");
    println!("mesh count-word high half: {extras:?}; normal-word flags: {nflags:?}");
    println!("slope classes (by recomputed normal): {slopes:?}");
    println!("zones with collision: {}", with_zone.join(" "));
}

fn open_zone(path: &str, root: &std::path::Path) -> (Zone, Collision) {
    let keys = ZoneKeys::from_install(root).expect("zone keys");
    let data = std::fs::read(path).expect("read DAT");
    let z = load_zone(&data, &keys).expect("no MZB in DAT");
    let c = Collision::parse(&z.mzb).expect("collision parse");
    (z, c)
}

fn bounds(tris: impl Iterator<Item = [V3; 3]>) -> (V3, V3) {
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for w in tris {
        for q in w {
            for k in 0..3 {
                lo[k] = lo[k].min(q[k]);
                hi[k] = hi[k].max(q[k]);
            }
        }
    }
    (lo, hi)
}

fn shot(path: &str, prefix: &str, size: usize, root: &std::path::Path) {
    let (z, col) = open_zone(path, root);
    let vis = visual_tris(&z);
    let (lo, hi) = bounds(vis.iter().map(|t| t.0));
    let span = (hi[0] - lo[0]).max(hi[2] - lo[2]).max(1.0);
    let vcol: Vec<_> = vis
        .iter()
        .map(|(p, ny)| {
            let g = (60.0 + 150.0 * ny) as u8;
            (*p, [g, g, g])
        })
        .collect();
    let mut ctris = Vec::new();
    col.for_each_triangle(|_, _, w, _| ctris.push((w, slope_color(normal_of(&w)[1]))));
    let (clo, chi) = bounds(ctris.iter().map(|t| t.0));
    println!("visual x {:.0}..{:.0} z {:.0}..{:.0}; collision x {:.0}..{:.0} z {:.0}..{:.0}", lo[0], hi[0], lo[2], hi[2], clo[0], chi[0], clo[2], chi[2]);
    for (name, t) in [("collision", &ctris), ("visual", &vcol)] {
        let rgba = raster(t, lo, span, size);
        std::fs::write(format!("{prefix}_{name}.png"), png::encode_rgba(size as u32, size as u32, &rgba)).expect("write png");
    }
    println!(
        "{} collision tris, {} visual tris -> {prefix}_collision.png, {prefix}_visual.png (frame x0 {:.1} z0 {:.1} span {:.1})",
        ctris.len(),
        vis.len(),
        lo[0],
        lo[2],
        span
    );
}

/// Highest visual surface y (smallest) at (x,z) among triangles whose x/z projection contains it.
fn visual_surfaces(vis: &[([V3; 3], f32)], x: f32, z: f32) -> Vec<f32> {
    let mut out = Vec::new();
    for (w, _) in vis {
        let area = (w[1][0] - w[0][0]) * (w[2][2] - w[0][2]) - (w[2][0] - w[0][0]) * (w[1][2] - w[0][2]);
        if area.abs() < 1e-9 {
            continue;
        }
        let l1 = ((x - w[0][0]) * (w[2][2] - w[0][2]) - (w[2][0] - w[0][0]) * (z - w[0][2])) / area;
        let l2 = ((w[1][0] - w[0][0]) * (z - w[0][2]) - (x - w[0][0]) * (w[1][2] - w[0][2])) / area;
        let l0 = 1.0 - l1 - l2;
        if l0 >= 0.0 && l1 >= 0.0 && l2 >= 0.0 {
            out.push(l0 * w[0][1] + l1 * w[1][1] + l2 * w[2][1]);
        }
    }
    out.sort_by(|a, b| a.total_cmp(b));
    out
}

fn heights(path: &str, root: &std::path::Path) {
    let (z, mut col) = open_zone(path, root);
    if let (Ok(x), Ok(zz)) = (std::env::var("SIGNET_OX"), std::env::var("SIGNET_OZ")) {
        col.origin = [x.parse().unwrap(), zz.parse().unwrap()];
    }
    let vis = visual_tris(&z);
    // deterministic sample grid over the collision's own extent
    let mut pts = Vec::new();
    let mut seed = 12345u64;
    let mut rnd = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as f32 / (1u64 << 31) as f32
    };
    let cells: Vec<_> = col.non_empty_cells().map(|(c, _)| c).collect();
    for _ in 0..400 {
        let c = cells[(rnd() * cells.len() as f32) as usize % cells.len()];
        let (cx, cz) = ((c % col.cols) as f32, (c / col.cols) as f32);
        pts.push((col.origin[0] + (cx + rnd()) * CELL, col.origin[1] + (cz + rnd()) * CELL));
    }
    // errors: min over visual surfaces of |visual - collision top|
    let (mut n, mut within, mut top_within, mut no_vis) = (0, 0, 0, 0);
    let mut errs = Vec::new();
    for (x, zz) in pts {
        let Some(h) = col.ground_height(x, zz) else { continue };
        n += 1;
        let vs = visual_surfaces(&vis, x, zz);
        if vs.is_empty() {
            no_vis += 1;
            continue;
        }
        let best = vs.iter().map(|v| (v - h).abs()).fold(f32::MAX, f32::min);
        errs.push(best);
        if best <= 0.5 {
            within += 1;
        }
        if (vs[0] - h).abs() <= 0.5 {
            top_within += 1;
        }
    }
    // visual-seeded: random points on visual triangles that face up; does collision have a surface there?
    let ups: Vec<_> = vis.iter().filter(|(w, _)| normal_of(w)[1].abs() > 0.7).collect();
    let (mut vn, mut vhit, mut vnone) = (0, 0, 0);
    for _ in 0..600 {
        if ups.is_empty() {
            break;
        }
        let w = &ups[(rnd() * ups.len() as f32) as usize % ups.len()].0;
        let (mut a, mut b) = (rnd(), rnd());
        if a + b > 1.0 {
            a = 1.0 - a;
            b = 1.0 - b;
        }
        let pt = |k: usize| w[0][k] + a * (w[1][k] - w[0][k]) + b * (w[2][k] - w[0][k]);
        let (x, y, zz) = (pt(0), pt(1), pt(2));
        vn += 1;
        let s = col.surfaces_at(x, zz);
        if s.is_empty() {
            vnone += 1;
        } else if s.iter().any(|h| (h - y).abs() <= 0.5) {
            vhit += 1;
        }
    }
    println!("visual-seeded points (upward visual faces): {vn}; collision has no surface there {vnone}; collision surface within 0.5 yalm of the visual point {vhit}");
    errs.sort_by(|a, b| a.total_cmp(b));
    let q = |f: f32| errs.get(((errs.len() as f32 - 1.0) * f) as usize).copied().unwrap_or(f32::NAN);
    println!("samples with collision {n}; no visual surface there {no_vis}");
    println!("collision top within 0.5 of ANY visual surface: {within}/{}; within 0.5 of the TOPMOST visual surface: {top_within}/{}", n - no_vis, n - no_vis);
    println!("min |visual - collision| quantiles: p50 {:.3}  p75 {:.3}  p90 {:.3}  p99 {:.3}", q(0.5), q(0.75), q(0.9), q(0.99));
}

fn info(path: &str, root: &std::path::Path) {
    let (z, col) = open_zone(path, root);
    let pl = crypt::placements(&z.mzb);
    println!("placements {}, instances {}, meshes {}, placed pairs {}, triangles {}", pl.len(), col.instances.len(), col.meshes.len(), col.placed.len(), col.triangle_count());
    let mut ctris = Vec::new();
    col.for_each_triangle(|_, _, w, _| ctris.push(w));
    let (lo, hi) = bounds(ctris.into_iter());
    println!("world bounds x {:.1}..{:.1}  y {:.1}..{:.1}  z {:.1}..{:.1}", lo[0], hi[0], lo[1], hi[1], lo[2], hi[2]);
    let mut by: BTreeMap<String, u64> = BTreeMap::new();
    for p in &col.placed {
        hist(&mut by, pl.get(p.instance as usize).map(|x| x.mesh.clone()).unwrap_or_default());
    }
    let mut v: Vec<_> = by.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    println!("most-used placements: {:?}", &v[..v.len().min(8)]);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root_of = |i: usize| args.get(i).map(PathBuf::from).or_else(find_install_root).expect("no install root");
    match args.first().map(String::as_str) {
        Some("census") => {
            let root = root_of(1);
            let keys = ZoneKeys::from_install(&root).expect("zone keys");
            census(&root, &keys)
        }
        Some("shot") if args.len() >= 3 => {
            let size = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(2048);
            shot(&args[1], &args[2], size, &root_of(4))
        }
        Some("heights") if args.len() >= 2 => heights(&args[1], &root_of(2)),
        Some("dump") if args.len() >= 3 => {
            let root = root_of(3);
            let keys = ZoneKeys::from_install(&root).expect("zone keys");
            let z = load_zone(&std::fs::read(&args[1]).expect("read"), &keys).expect("no MZB");
            std::fs::write(&args[2], &z.mzb).expect("write");
        }
        Some("fit") if args.len() >= 2 => {
            let (_, col) = open_zone(&args[1], &root_of(2));
            // Vote for the grid origin using meshes whose bounds fit inside one cell.
            let mut vx = vec![0u32; 241];
            let mut vz = vec![0u32; 241];
            let mut used = 0;
            let (cx0, cz0) = (col.origin[0], col.origin[1]);
            for (c, items) in col.non_empty_cells() {
                let (cx, cz) = ((c % col.cols) as f32, (c / col.cols) as f32);
                for &pi in items {
                    let m = &col.meshes[col.placed[pi as usize].mesh as usize];
                    let mut ws = Vec::new();
                    for ti in 0..m.tris.len() {
                        ws.extend(col.world_tri(pi, ti as u32));
                    }
                    if ws.is_empty() {
                        continue;
                    }
                    let b = |k: usize| (ws.iter().map(|q| q[k]).fold(f32::MAX, f32::min), ws.iter().map(|q| q[k]).fold(f32::MIN, f32::max));
                    let (bx, bz) = (b(0), b(2));
                    if bx.1 - bx.0 > CELL - 0.2 || bz.1 - bz.0 > CELL - 0.2 {
                        continue;
                    }
                    used += 1;
                    for (v, bb, cc, o0) in [(&mut vx, bx, cx, cx0), (&mut vz, bz, cz, cz0)] {
                        for (i, slot) in v.iter_mut().enumerate() {
                            let o = o0 - 6.0 + i as f32 * 0.05;
                            if bb.0 >= o + cc * CELL - 0.02 && bb.1 <= o + (cc + 1.0) * CELL + 0.02 {
                                *slot += 1;
                            }
                        }
                    }
                }
            }
            let best = |v: &Vec<u32>, o0: f32| {
                let (i, n) = v.iter().enumerate().max_by_key(|(_, n)| **n).unwrap();
                (o0 - 6.0 + i as f32 * 0.05, *n)
            };
            let (bx, bz) = (best(&vx, cx0), best(&vz, cz0));
            println!("grid {}x{} centered {:?}; small meshes {used}; best origin x {:.2} ({} votes) z {:.2} ({} votes)", col.cols, col.rows, col.origin, bx.0, bx.1, bz.0, bz.1);
        }
        Some("info") if args.len() >= 2 => info(&args[1], &root_of(2)),
        _ => eprintln!("usage: signet-dat-collision census|shot|heights|info ... (see source header)"),
    }
}
