//! Install-wide census of actor data: skeletons (0x29), actor meshes (0x2A) and
//! animations (0x2B). Counts chunks, decodes every one, tallies failures by reason
//! and runs sanity checks (parents, finite vertices, bone indexes, unit quaternions).
//! Usage: signet-dat-actors [install-root] [--list]
//!   --list  also print one line per DAT that holds a skeleton or actor mesh.

use std::collections::BTreeMap;
use std::path::PathBuf;

use signet_dat::{Install, actor_mesh, anim, chunks, find_install_root, skeleton};

#[derive(Default)]
struct Totals {
    chunks: [u64; 3],
    decoded: [u64; 3],
    failures: BTreeMap<String, u64>,
    bones: u64,
    vertices: u64,
    triangles: u64,
    frames: u64,
    tracks: u64,
    cloth_meshes: u64,
    dual_vertices: u64,
    // sanity
    bad_vertex_values: u64,
    mesh_bone_out_of_range: u64,
    meshes_checked_against_skeleton: u64,
    quat_not_unit: u64,
    quats_checked: u64,
    anim_bone_out_of_range: u64,
    anims_checked_against_skeleton: u64,
    anim_bad_values: u64,
    weights_off: u64,
    files_with_actor_data: u64,
    files_read_errors: u64,
    chunk_errors: u64,
    largest_skeleton: usize,
    max_frames: u16,
}

fn reason(prefix: &str, e: &dyn std::fmt::Debug) -> String {
    let s = format!("{e:?}");
    let name = s.split(|c: char| !c.is_alphanumeric()).next().unwrap_or("");
    format!("{prefix}: {name}")
}

fn finite(v: &[f32]) -> bool {
    v.iter().all(|x| x.is_finite() && x.abs() < 1.0e6)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let list = args.iter().any(|a| a == "--list");
    let root = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map(PathBuf::from)
        .or_else(find_install_root)
        .expect("no install root (pass a path or set SIGNET_FFXI_ROOT)");
    let install = Install::open(&root).expect("open install");
    let mut t = Totals::default();
    let mut ids_seen = 0u64;
    let mut seen_files = std::collections::HashSet::new();
    let mut examples: Vec<String> = Vec::new();

    for id in 0..install.id_count() {
        let Some(loc) = install.locate(id) else { continue };
        // Many ids share one DAT; count each file once.
        if !seen_files.insert(loc) {
            continue;
        }
        let Ok(dat) = std::fs::read(install.path(loc)) else {
            t.files_read_errors += 1;
            continue;
        };
        ids_seen += 1;
        let mut skeletons: Vec<skeleton::Skeleton> = Vec::new();
        let (mut n_sk, mut n_mesh, mut n_anim) = (0, 0, 0);
        let mut meshes: Vec<actor_mesh::ActorMesh> = Vec::new();
        let mut anims: Vec<anim::Animation> = Vec::new();
        let mut it = chunks(&dat);
        for c in it.by_ref() {
            let Ok(c) = c else {
                t.chunk_errors += 1;
                break;
            };
            match c.kind {
                skeleton::KIND => {
                    n_sk += 1;
                    t.chunks[0] += 1;
                    match skeleton::parse(c.data) {
                        Ok(s) => {
                            t.decoded[0] += 1;
                            t.bones += s.bones.len() as u64;
                            t.largest_skeleton = t.largest_skeleton.max(s.bones.len());
                            for b in &s.bones {
                                t.quats_checked += 1;
                                let n: f32 = b.rotation.iter().map(|x| x * x).sum::<f32>().sqrt();
                                if !(0.98..=1.02).contains(&n) || !finite(&b.rotation) || !finite(&b.translation) {
                                    t.quat_not_unit += 1;
                                }
                            }
                            skeletons.push(s);
                        }
                        Err(e) => {
                            *t.failures.entry(reason("skeleton", &e)).or_default() += 1;
                            examples.push(format!("id {id} {}: skeleton {} {e}", loc.relative_path().display(), c.name_str()));
                        }
                    }
                }
                actor_mesh::KIND => {
                    n_mesh += 1;
                    t.chunks[1] += 1;
                    match actor_mesh::parse(c.data) {
                        Ok(m) => {
                            t.decoded[1] += 1;
                            t.vertices += m.vertices.len() as u64;
                            t.triangles += m.groups.iter().map(|g| g.corners.len() as u64 / 3).sum::<u64>();
                            t.cloth_meshes += m.cloth as u64;
                            for v in &m.vertices {
                                if v.influences.len() == 2 {
                                    t.dual_vertices += 1;
                                    let w = v.influences[0].weight + v.influences[1].weight;
                                    if (w - 1.0).abs() > 0.02 {
                                        t.weights_off += 1;
                                    }
                                }
                                for i in &v.influences {
                                    if !finite(&i.position) || !finite(&i.normal) || !i.weight.is_finite() {
                                        t.bad_vertex_values += 1;
                                        if examples.len() < 40 {
                                            examples.push(format!("id {id} {}: mesh {} non-finite vertex", loc.relative_path().display(), c.name_str()));
                                        }
                                    }
                                }
                            }
                            for g in &m.groups {
                                if g.corners.iter().any(|c| !finite(&c.uv)) {
                                    t.bad_vertex_values += 1;
                                }
                            }
                            meshes.push(m);
                        }
                        Err(e) => {
                            *t.failures.entry(reason("mesh", &e)).or_default() += 1;
                            examples.push(format!("id {id} {}: mesh {} {e}", loc.relative_path().display(), c.name_str()));
                        }
                    }
                }
                anim::KIND => {
                    n_anim += 1;
                    t.chunks[2] += 1;
                    match anim::parse(c.data) {
                        Ok(a) => {
                            t.decoded[2] += 1;
                            t.frames += a.frames as u64;
                            t.tracks += a.tracks.len() as u64;
                            t.max_frames = t.max_frames.max(a.frames);
                            for tr in &a.tracks {
                                let chans = tr.rotation.iter().chain(&tr.translation).chain(&tr.scale);
                                let bad = chans.into_iter().any(|ch| match ch {
                                    anim::Channel::Const(v) => !v.is_finite() || v.abs() > 1.0e6,
                                    anim::Channel::Keys(k) => !finite(k),
                                });
                                if bad {
                                    t.anim_bad_values += 1;
                                    if examples.len() < 40 {
                                        examples.push(format!("id {id} {}: anim {} bad values", loc.relative_path().display(), c.name_str()));
                                    }
                                }
                            }
                            anims.push(a);
                        }
                        Err(e) => *t.failures.entry(reason("anim", &e)).or_default() += 1,
                    }
                }
                _ => {}
            }
        }
        if n_sk + n_mesh + n_anim > 0 {
            t.files_with_actor_data += 1;
            if list && (n_sk > 0 || n_mesh > 0) {
                let bones = skeletons.first().map_or(0, |s| s.bones.len());
                let verts: usize = meshes.iter().map(|m| m.vertices.len()).sum();
                println!(
                    "id {id:6} {}  skeletons {n_sk} (bones {bones}) meshes {n_mesh} (vertices {verts}) anims {n_anim}",
                    loc.relative_path().display()
                );
            }
        }
        // Cross-checks need the skeleton that lives in the same DAT.
        if let Some(sk) = skeletons.first() {
            let nb = sk.bones.len() as u16;
            for m in &meshes {
                t.meshes_checked_against_skeleton += 1;
                let bad = m.vertices.iter().flat_map(|v| &v.influences).any(|i| i.bone >= nb || i.bone_mirror >= nb);
                if bad {
                    t.mesh_bone_out_of_range += 1;
                }
            }
            for a in &anims {
                t.anims_checked_against_skeleton += 1;
                if a.tracks.iter().any(|tr| tr.bone >= sk.bones.len()) {
                    t.anim_bone_out_of_range += 1;
                }
            }
        }
    }

    let k = ["skeletons (0x29)", "actor meshes (0x2A)", "animations (0x2B)"];
    println!("install {}  distinct DAT files read {ids_seen}  files with actor data {}", root.display(), t.files_with_actor_data);
    for i in 0..3 {
        println!("{:20} chunks {:7}  decoded {:7}  failed {}", k[i], t.chunks[i], t.decoded[i], t.chunks[i] - t.decoded[i]);
    }
    if t.failures.is_empty() {
        println!("failures: none");
    } else {
        println!("failures by reason:");
        for (r, n) in &t.failures {
            println!("  {r}: {n}");
        }
    }
    println!(
        "totals: bones {} (largest skeleton {})  vertices {} (dual-influence {}, cloth meshes {})  triangles {}",
        t.bones, t.largest_skeleton, t.vertices, t.dual_vertices, t.cloth_meshes, t.triangles
    );
    println!("        anim tracks {}  frames {} (longest {})", t.tracks, t.frames, t.max_frames);
    println!("sanity:");
    println!("  skeleton parent indexes valid: enforced by decoder (BadParent failures above)");
    println!("  bone rotations not unit length or non-finite: {} of {}", t.quat_not_unit, t.quats_checked);
    println!("  vertex/uv/weight values non-finite or huge: {}", t.bad_vertex_values);
    println!("  dual vertices with weights not summing to 1: {} of {}", t.weights_off, t.dual_vertices);
    println!(
        "  meshes with a bone index past their DAT's skeleton: {} of {} checked",
        t.mesh_bone_out_of_range, t.meshes_checked_against_skeleton
    );
    println!(
        "  animations with a track past their DAT's skeleton: {} of {} checked",
        t.anim_bone_out_of_range, t.anims_checked_against_skeleton
    );
    println!("  animation tracks with non-finite values: {}", t.anim_bad_values);
    for e in examples.iter().take(25) {
        println!("  example: {e}");
    }
    println!("  unreadable files {}  chunk-stream errors {}", t.files_read_errors, t.chunk_errors);
}
