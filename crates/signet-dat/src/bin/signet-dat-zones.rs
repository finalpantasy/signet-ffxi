//! Decodes every zone layout (MZB) and zone mesh (MMB) chunk in the install and
//! reports how many decode, plus entropy before and after as a sanity check.
//! Usage: signet-dat-zones [--show <path-to-.DAT>] [install-root]

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use signet_dat::crypt::{self, ZoneKeys};
use signet_dat::{Install, chunks, find_install_root, mmb};

fn entropy(b: &[u8]) -> f64 {
    if b.is_empty() {
        return 0.0;
    }
    let mut n = [0u64; 256];
    b.iter().for_each(|&x| n[x as usize] += 1);
    let len = b.len() as f64;
    n.iter().filter(|&&c| c > 0).map(|&c| -(c as f64 / len) * (c as f64 / len).log2()).sum()
}

#[derive(Default)]
struct Tally {
    results: BTreeMap<String, u64>,
    bytes: u64,
    before: f64,
    after: f64,
}

fn show(path: &str, keys: &ZoneKeys) {
    let data = std::fs::read(path).expect("read DAT");
    let mut mmb_names = HashSet::new();
    // The layout chunk can come before the meshes it names, so collect names first.
    for c in chunks(&data).filter_map(Result::ok).filter(|c| c.kind == crypt::MMB_KIND) {
        let mut p = c.data.to_vec();
        crypt::decode_mmb(&mut p, keys);
        if let Some(n) = p.get(16..32) {
            mmb_names.insert(String::from_utf8_lossy(n).trim_end_matches([' ', '\0']).to_string());
        }
    }
    for c in chunks(&data).filter_map(Result::ok) {
        let mut p = c.data.to_vec();
        match c.kind {
            crypt::MMB_KIND => {
                crypt::decode_mmb(&mut p, keys);
                if mmb_names.len() < 4 {
                    println!("MMB {:<4} {:>7} bytes  head {:02x?}", c.name_str(), p.len(), &p[..48.min(p.len())]);
                }
                if let Some(n) = p.get(16..32) {
                    mmb_names.insert(String::from_utf8_lossy(n).trim_end_matches([' ', '\0']).to_string());
                }
            }
            crypt::MZB_KIND => {
                crypt::decode_mzb(&mut p, keys);
                let pl = crypt::placements(&p);
                println!("MZB {:<4} {} bytes, {} placements", c.name_str(), p.len(), pl.len());
                for x in pl.iter().take(8) {
                    println!("  {:<16} pos {:?} rot {:?} scale {:?}", x.mesh, x.position, x.rotation, x.scale);
                }
                let names: HashSet<_> = pl.iter().map(|x| x.mesh.clone()).collect();
                let missing: Vec<_> = names.iter().filter(|n| !mmb_names.contains(*n)).collect();
                println!("  distinct mesh names: {}  not found among this file's MMBs: {}", names.len(), missing.len());
                for m in missing.iter().take(5) {
                    println!("    missing: {m}");
                }
                let mut have: Vec<_> = mmb_names.iter().collect();
                have.sort();
                println!("  MMB names seen so far ({}): {:?}", have.len(), &have[..have.len().min(12)]);
            }
            _ => {}
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let show_path = args.iter().position(|a| a == "--show").and_then(|i| args.get(i + 1)).cloned();
    let root = args
        .iter()
        .filter(|a| Some(*a) != show_path.as_ref() && !a.starts_with("--"))
        .map(PathBuf::from)
        .next()
        .or_else(find_install_root)
        .expect("no install root");
    let keys = ZoneKeys::from_install(&root).expect("zone keys from FFXiMain.dll");
    if let Some(p) = show_path {
        show(&p, &keys);
        return;
    }
    let install = Install::open(&root).expect("open install");
    let start = Instant::now();
    let mut seen = HashSet::new();
    let mut tallies: BTreeMap<&str, Tally> = BTreeMap::new();
    let mut placements = 0u64;
    let mut mesh_results: BTreeMap<String, u64> = BTreeMap::new();
    let mut mesh_examples: Vec<String> = Vec::new();
    let (mut verts, mut tris, mut outside) = (0u64, 0u64, 0u64);
    for id in 0..install.id_count() {
        let Some(loc) = install.locate(id) else { continue };
        if !seen.insert(loc) {
            continue;
        }
        let Ok(data) = std::fs::read(install.path(loc)) else { continue };
        for c in chunks(&data).filter_map(Result::ok) {
            let (label, r, p) = match c.kind {
                crypt::MMB_KIND => {
                    let mut p = c.data.to_vec();
                    let r = crypt::decode_mmb(&mut p, &keys);
                    let key = match mmb::parse(&p) {
                        Ok(m) => {
                            for s in &m.submeshes {
                                verts += s.vertices.len() as u64;
                                tris += s.indices.len() as u64 / 3;
                                let slack = |a: [f32; 2]| (a[1] - a[0]).abs() * 0.01 + 0.01;
                                outside += s
                                    .vertices
                                    .iter()
                                    .filter(|v| {
                                        (0..3).any(|k| {
                                            let b = m.bounds[k];
                                            v.position[k] < b[0] - slack(b) || v.position[k] > b[1] + slack(b)
                                        })
                                    })
                                    .count() as u64;
                            }
                            "ok".to_string()
                        }
                        Err(e) => {
                            if mesh_examples.len() < 8 {
                                mesh_examples.push(format!("{:?} {}: {e} (head {:02x?})", loc.relative_path(), c.name_str(), &p[..8.min(p.len())]));
                            }
                            match e {
                                mmb::MmbError::Truncated { .. } => "truncated".into(),
                                mmb::MmbError::TooManySubMeshes(_) => "bad sub-mesh count".into(),
                                mmb::MmbError::IndexOutOfRange { .. } => "index out of range".into(),
                            }
                        }
                    };
                    *mesh_results.entry(key).or_default() += 1;
                    ("MMB", r, p)
                }
                crypt::MZB_KIND => {
                    let mut p = c.data.to_vec();
                    let r = crypt::decode_mzb(&mut p, &keys);
                    placements += crypt::placements(&p).len() as u64;
                    ("MZB", r, p)
                }
                _ => continue,
            };
            let t = tallies.entry(label).or_default();
            *t.results.entry(format!("{r:?}")).or_default() += 1;
            let w = c.data.len() as f64;
            t.bytes += c.data.len() as u64;
            t.before += entropy(c.data) * w;
            t.after += entropy(&p) * w;
        }
    }
    for (label, t) in &tallies {
        let w = t.bytes.max(1) as f64;
        println!(
            "{label}: {:?}  bytes {}  entropy before {:.2} after {:.2} bits/byte",
            t.results,
            t.bytes,
            t.before / w,
            t.after / w
        );
    }
    println!("MZB placements: {placements}");
    println!("MMB geometry: {mesh_results:?}  vertices {verts}  triangles {tris}  vertices outside bounds {outside}");
    for e in &mesh_examples {
        println!("  example: {e}");
    }
    println!("elapsed: {:.1}s", start.elapsed().as_secs_f64());
}
