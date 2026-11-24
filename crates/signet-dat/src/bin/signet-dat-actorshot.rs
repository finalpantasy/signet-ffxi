//! Renders a character DAT with a small software rasterizer: the skeleton in bind
//! pose (or one animation frame), skinned actor meshes, textured with the DAT's own
//! textures. Writes <prefix>_front.png and <prefix>_side.png (side by side in one
//! image per view set when --sheet is used).
//! Usage: signet-dat-actorshot (<file.DAT> | --id N) <out-prefix>
//!        [--parts a,b,c] [--anim NAME] [--frame F] [--size PX] [--flat] [--list]
//!   --parts  only mesh chunks with these names (default: all)
//!   --anim   pose with the animation chunk of that name at frame F
//!   --list   print chunk names and exit

use std::collections::HashMap;
use std::path::PathBuf;

use signet_dat::skeleton::{Pose, Skeleton};
use signet_dat::texture::{self, Image};
use signet_dat::{Install, actor_mesh, anim, chunks, find_install_root, png, skeleton};

type V3 = [f32; 3];

struct Tri {
    p: [V3; 3],
    uv: [[f32; 2]; 3],
    shade: f32,
    tex: Option<usize>,
}

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flat = args.iter().any(|a| a == "--flat");
    let list = args.iter().any(|a| a == "--list");
    let parts: Option<Vec<String>> = arg_value(&args, "--parts").map(|s| s.split(',').map(str::to_string).collect());
    let anim_name = arg_value(&args, "--anim");
    let frame: f32 = arg_value(&args, "--frame").and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let size: usize = arg_value(&args, "--size").and_then(|s| s.parse().ok()).unwrap_or(900);
    // Positional arguments: [file.DAT] <out-prefix>; flags that take a value are skipped.
    let valued = ["--parts", "--anim", "--frame", "--size", "--id", "--skel-id", "--mesh-ids"];
    let mut positional: Vec<&String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if valued.contains(&args[i].as_str()) {
            i += 2;
        } else if args[i].starts_with("--") {
            i += 1;
        } else {
            positional.push(&args[i]);
            i += 1;
        }
    }
    let dat = if let Some(id) = arg_value(&args, "--id") {
        let root = find_install_root().expect("no install root");
        Install::open(root).expect("open install").read(id.parse().expect("id")).expect("read id")
    } else {
        std::fs::read(positional.first().expect("usage: <file.DAT> <out-prefix>")).expect("read DAT")
    };
    // Extra DATs merged in: --skel-id N supplies the skeleton and animations (player body parts ship
    // without them), --mesh-ids a,b,c adds more mesh DATs (equipment slots) and their textures.
    let mut dats: Vec<Vec<u8>> = vec![dat];
    if args.iter().any(|a| a == "--skel-id" || a == "--mesh-ids") {
        let install = Install::open(find_install_root().expect("no install root")).expect("open install");
        let mut extra: Vec<u32> = arg_value(&args, "--skel-id").iter().map(|s| s.parse().expect("id")).collect();
        extra.extend(arg_value(&args, "--mesh-ids").iter().flat_map(|s| s.split(',').map(|x| x.parse::<u32>().expect("id")).collect::<Vec<_>>()));
        for id in extra {
            dats.push(install.read(id).expect("read id"));
        }
    }
    let prefix = positional.last().map(|s| s.to_string()).unwrap_or_else(|| "actor".into());

    let mut images: Vec<Image> = Vec::new();
    let mut tex_by_name: HashMap<String, usize> = HashMap::new();
    let mut skel: Option<Skeleton> = None;
    let mut meshes: Vec<(String, actor_mesh::ActorMesh)> = Vec::new();
    let mut anims: Vec<(String, anim::Animation)> = Vec::new();
    for c in dats.iter().flat_map(|d| chunks(d)).filter_map(Result::ok) {
        if list {
            println!("{} kind 0x{:02x} len {}", c.name_str(), c.kind, c.data.len());
        }
        match c.kind {
            texture::KIND => {
                if let Ok((h, img)) = texture::decode(c.data) {
                    tex_by_name.insert(format!("{:<8}{:<8}", h.group, h.name).trim_end().to_string(), images.len());
                    tex_by_name.entry(h.name.clone()).or_insert(images.len());
                    images.push(img);
                }
            }
            skeleton::KIND => {
                if skel.is_none() {
                    skel = skeleton::parse(c.data).ok();
                }
            }
            actor_mesh::KIND => {
                if let Ok(m) = actor_mesh::parse(c.data) {
                    meshes.push((c.name_str().trim_end_matches('.').to_string(), m));
                }
            }
            anim::KIND => {
                if let Ok(a) = anim::parse(c.data) {
                    anims.push((c.name_str().trim_end_matches('.').to_string(), a));
                }
            }
            _ => {}
        }
    }
    if list {
        return;
    }
    let skel = skel.expect("DAT has no skeleton");
    let local = skel.bind_local();
    let (local, posed) = match &anim_name {
        Some(n) => {
            let a = &anims.iter().find(|(name, _)| name == n).unwrap_or_else(|| panic!("no animation {n}")).1;
            (a.local_poses(&local, frame), format!("animation {n} frame {frame}"))
        }
        None => (local, "bind pose".to_string()),
    };
    let world: Vec<Pose> = skel.world_from_local(&local);

    let mut tris: Vec<Tri> = Vec::new();
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    let mut used = Vec::new();
    for (name, m) in &meshes {
        if let Some(p) = &parts {
            if !p.contains(name) {
                continue;
            }
        }
        used.push(name.clone());
        let twins: &[bool] = if m.flip { &[false, true] } else { &[false] };
        for &mirrored in twins {
            let skinned = m.skin(&world, mirrored);
            for g in &m.groups {
                let tex = tex_by_name.get(&g.texture).copied();
                for t in g.corners.chunks_exact(3) {
                    let p = [0, 1, 2].map(|k| skinned[t[k].vertex as usize].0);
                    let n = skinned[t[0].vertex as usize].1;
                    for q in &p {
                        for k in 0..3 {
                            lo[k] = lo[k].min(q[k]);
                            hi[k] = hi[k].max(q[k]);
                        }
                    }
                    tris.push(Tri { p, uv: [0, 1, 2].map(|k| t[k].uv), shade: 0.55 + 0.45 * n[2].abs().max(n[0].abs() * 0.7).min(1.0), tex });
                }
            }
        }
    }
    println!(
        "{posed}: {} bones, meshes used {:?}, triangles {}, textures {}",
        skel.bones.len(),
        used,
        tris.len(),
        images.len()
    );
    println!("extent x {:.2}..{:.2}  y {:.2}..{:.2}  z {:.2}..{:.2}", lo[0], hi[0], lo[1], hi[1], lo[2], hi[2]);

    // FFXI's y axis points down, so screen-up is -y. Player models face +x in bind pose: the front
    // view looks along -x (-z to the right), the side view looks along +z (x to the right).
    let views: [(&str, fn(V3) -> [f32; 3]); 2] = [
        ("front", |p| [-p[2], -p[1], -p[0]]),
        ("side", |p| [p[0], -p[1], p[2]]),
    ];
    for (label, proj) in views {
        let pts: Vec<[V3; 3]> = tris.iter().map(|t| t.p.map(proj)).collect();
        let (mut vlo, mut vhi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for t in &pts {
            for q in t {
                for k in 0..2 {
                    vlo[k] = vlo[k].min(q[k]);
                    vhi[k] = vhi[k].max(q[k]);
                }
            }
        }
        let span = (vhi[0] - vlo[0]).max(vhi[1] - vlo[1]).max(1e-3);
        let scale = (size as f32 - 40.0) / span;
        let ox = (size as f32 - (vhi[0] - vlo[0]) * scale) / 2.0;
        let oy = (size as f32 - (vhi[1] - vlo[1]) * scale) / 2.0;
        let mut color = vec![[30u8, 30, 40, 255]; size * size];
        let mut depth = vec![f32::MAX; size * size];
        for (t, q) in tris.iter().zip(&pts) {
            let s = q.map(|v| [ox + (v[0] - vlo[0]) * scale, oy + (vhi[1] - v[1]) * scale, v[2]]);
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
                    if z >= depth[i] {
                        continue;
                    }
                    let mut rgb = [170.0f32, 170.0, 170.0];
                    if let Some(ti) = t.tex.filter(|_| !flat) {
                        let img = &images[ti];
                        let u = w0 * t.uv[0][0] + w1 * t.uv[1][0] + w2 * t.uv[2][0];
                        let v = w0 * t.uv[0][1] + w1 * t.uv[1][1] + w2 * t.uv[2][1];
                        let tx = (u.rem_euclid(1.0) * img.width as f32) as usize % img.width as usize;
                        let ty = (v.rem_euclid(1.0) * img.height as f32) as usize % img.height as usize;
                        let c = &img.rgba[(ty * img.width as usize + tx) * 4..][..4];
                        if c[3] < 8 {
                            continue;
                        }
                        rgb = [c[0] as f32, c[1] as f32, c[2] as f32];
                    }
                    depth[i] = z;
                    color[i] = [(rgb[0] * t.shade) as u8, (rgb[1] * t.shade) as u8, (rgb[2] * t.shade) as u8, 255];
                }
            }
        }
        let rgba: Vec<u8> = color.into_iter().flatten().collect();
        let out = PathBuf::from(format!("{prefix}_{label}.png"));
        std::fs::write(&out, png::encode_rgba(size as u32, size as u32, &rgba)).expect("write png");
        println!("-> {}", out.display());
    }
}
