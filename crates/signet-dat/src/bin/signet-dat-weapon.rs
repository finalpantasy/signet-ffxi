//! Prints what a weapon DAT holds (chunks, mesh bone tables, vertex extents per bone).
//! Usage: signet-dat-weapon <race 1..8> <main-model-id> [--slot 6] [--root <install>]
use signet_dat::modelmap::equip_file;
use signet_dat::{Install, actor_mesh, chunks, find_install_root, skeleton};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let root = a.iter().position(|x| x == "--root").and_then(|i| a.get(i + 1)).map(Into::into).or_else(find_install_root).expect("no install root");
    let slot: u8 = a.iter().position(|x| x == "--slot").and_then(|i| a.get(i + 1)).and_then(|s| s.parse().ok()).unwrap_or(6);
    let race: u8 = a[0].parse().unwrap();
    let id: u16 = a[1].parse().unwrap();
    let file = equip_file(race, slot, id).expect("no file");
    println!("race {race} slot {slot} model {id} -> file {file}");
    let dat = Install::open(&root).expect("open install").read(file).expect("read");
    for c in chunks(&dat).filter_map(Result::ok) {
        println!("chunk kind {:#04x} name {:?} len {}", c.kind, c.name_str(), c.data.len());
        if c.kind == skeleton::KIND {
            if let Ok(s) = skeleton::parse(c.data) {
                println!("  skeleton: {} bones", s.bones.len());
                for (i, b) in s.bones.iter().enumerate().take(12) {
                    println!("   bone {i} parent {:?} t {:?}", b.parent, b.translation);
                }
            }
        }
        if c.kind == actor_mesh::KIND {
            let p = c.data;
            let u16at = |o: usize| u16::from_le_bytes([p[o], p[o + 1]]) as usize;
            let (ty, flip, tbl_at, tbl_n) = (u16at(2), u16at(4), u16at(0x0C) * 2, u16at(0x10));
            let table: Vec<usize> = (0..tbl_n).map(|i| u16at(tbl_at + 2 * i)).collect();
            println!("  mesh type {ty:#x} flip {flip} bone table {table:?}");
            if let Ok(m) = actor_mesh::parse(p) {
                let mut per_bone: std::collections::BTreeMap<u16, ([f32; 3], [f32; 3], usize)> = Default::default();
                for v in &m.vertices {
                    for inf in &v.influences {
                        let e = per_bone.entry(inf.bone).or_insert(([f32::MAX; 3], [f32::MIN; 3], 0));
                        for k in 0..3 {
                            e.0[k] = e.0[k].min(inf.position[k]);
                            e.1[k] = e.1[k].max(inf.position[k]);
                        }
                        e.2 += 1;
                    }
                }
                println!("  vertices {} groups {} (tris {})", m.vertices.len(), m.groups.len(), m.groups.iter().map(|g| g.corners.len() / 3).sum::<usize>());
                let mut zs: Vec<(usize, f32)> = m.vertices.iter().enumerate().map(|(i, v)| (i, v.influences[0].position[2])).collect();
                zs.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                println!("  lowest z vertices: {:?}", zs.iter().take(8).collect::<Vec<_>>());
                println!("  z quantiles: {:?}", [0, 10, 25, 50, 75, 90, 100].map(|q| zs[(zs.len() - 1) * q / 100].1));
                for g in &m.groups {
                    println!("  group tex {:?} tris {}", g.texture, g.corners.len() / 3);
                    for t in g.corners.chunks(3) {
                        let zz: Vec<f32> = t.iter().map(|c| m.vertices[c.vertex as usize].influences[0].position[2]).collect();
                        if zz.iter().any(|&z| z < -0.4) {
                            let ps: Vec<[f32; 3]> = t.iter().map(|c| m.vertices[c.vertex as usize].influences[0].position).collect();
                            println!("   tri verts {:?} pos {:?} uv {:?}", t.iter().map(|c| c.vertex).collect::<Vec<_>>(), ps, t.iter().map(|c| c.uv).collect::<Vec<_>>());
                        }
                    }
                }
                for (b, (lo, hi, n)) in per_bone {
                    println!("   bone {b}: {n} influences, local x {:.3}..{:.3} y {:.3}..{:.3} z {:.3}..{:.3}", lo[0], hi[0], lo[1], hi[1], lo[2], hi[2]);
                }
            }
        }
    }
}
