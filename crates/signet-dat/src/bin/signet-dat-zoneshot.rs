//! Renders a top-down picture of one zone DAT with a small software rasterizer:
//! MZB placements of MMB meshes, textured with the DAT's own textures.
//! An end-to-end check of the zone decoders before the GPU viewer exists.
//! Usage: signet-dat-zoneshot <zone .DAT> <out.png> [size] [install-root]

use std::collections::HashMap;
use std::path::PathBuf;

use signet_dat::crypt::{self, ZoneKeys};
use signet_dat::texture::{self, Image};
use signet_dat::{chunks, find_install_root, mmb, png};

type V3 = [f32; 3];

fn rotate(v: V3, r: V3) -> V3 {
    let (sx, cx) = r[0].sin_cos();
    let (sy, cy) = r[1].sin_cos();
    let (sz, cz) = r[2].sin_cos();
    let v = [v[0], v[1] * cx - v[2] * sx, v[1] * sx + v[2] * cx];
    let v = [v[0] * cy + v[2] * sy, v[1], -v[0] * sy + v[2] * cy];
    [v[0] * cz - v[1] * sz, v[0] * sz + v[1] * cz, v[2]]
}

struct Tri {
    p: [V3; 3],
    uv: [[f32; 2]; 3],
    shade: f32,
    tex: Option<usize>,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dat = std::fs::read(&args[0]).expect("read zone DAT");
    let out = PathBuf::from(&args[1]);
    let size: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2048);
    let root = args.get(3).map(PathBuf::from).or_else(find_install_root).expect("no install root");
    let keys = ZoneKeys::from_install(&root).expect("zone keys");

    let mut images: Vec<Image> = Vec::new();
    let mut tex_by_name: HashMap<String, usize> = HashMap::new();
    let mut meshes: HashMap<String, mmb::Mesh> = HashMap::new();
    let mut layout = Vec::new();
    for c in chunks(&dat).filter_map(Result::ok) {
        match c.kind {
            texture::KIND => {
                if let Ok((h, img)) = texture::decode(c.data) {
                    tex_by_name.insert(format!("{:<8}{:<8}", h.group, h.name).trim_end().to_string(), images.len());
                    tex_by_name.entry(h.name.clone()).or_insert(images.len());
                    images.push(img);
                }
            }
            crypt::MMB_KIND => {
                let mut p = c.data.to_vec();
                crypt::decode_mmb(&mut p, &keys);
                if let Ok(m) = mmb::parse(&p) {
                    meshes.insert(m.name.clone(), m);
                }
            }
            crypt::MZB_KIND => {
                let mut p = c.data.to_vec();
                crypt::decode_mzb(&mut p, &keys);
                layout = crypt::placements(&p);
            }
            _ => {}
        }
    }

    let mut tris = Vec::new();
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    let (mut tex_hit, mut tex_miss) = (0u64, 0u64);
    for pl in &layout {
        let Some(m) = meshes.get(&pl.mesh) else { continue };
        for s in &m.submeshes {
            let tex = tex_by_name.get(&s.texture).copied();
            if tex.is_some() { tex_hit += 1 } else { tex_miss += 1 }
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
                let p = idx.map(|i| world[i]);
                for q in &p {
                    for k in 0..3 {
                        lo[k] = lo[k].min(q[k]);
                        hi[k] = hi[k].max(q[k]);
                    }
                }
                let n = rotate(s.vertices[idx[0]].normal, pl.rotation);
                tris.push(Tri { p, uv: idx.map(|i| s.vertices[i].uv), shade: 0.45 + 0.55 * n[1].abs().min(1.0), tex });
            }
        }
    }

    // Orthographic top-down: x right, z down the image; height is -y (FFXI's y points down).
    let span = (hi[0] - lo[0]).max(hi[2] - lo[2]).max(1.0);
    let scale = (size - 1) as f32 / span;
    let mut color = vec![[24u8, 24, 32, 255]; size * size];
    let mut depth = vec![f32::MAX; size * size];
    for t in &tris {
        let s = t.p.map(|q| [(q[0] - lo[0]) * scale, (q[2] - lo[2]) * scale, q[1]]);
        let area = (s[1][0] - s[0][0]) * (s[2][1] - s[0][1]) - (s[2][0] - s[0][0]) * (s[1][1] - s[0][1]);
        if area.abs() < 1e-6 {
            continue;
        }
        let x0 = s.iter().map(|q| q[0]).fold(f32::MAX, f32::min).max(0.0) as usize;
        let x1 = (s.iter().map(|q| q[0]).fold(f32::MIN, f32::max) as usize).min(size - 1);
        let y0 = s.iter().map(|q| q[1]).fold(f32::MAX, f32::min).max(0.0) as usize;
        let y1 = (s.iter().map(|q| q[1]).fold(f32::MIN, f32::max) as usize).min(size - 1);
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
                let mut rgb = [150.0f32, 150.0, 150.0];
                if let Some(ti) = t.tex.filter(|_| std::env::var_os("SIGNET_FLAT").is_none()) {
                    let img = &images[ti];
                    let u = w0 * t.uv[0][0] + w1 * t.uv[1][0] + w2 * t.uv[2][0];
                    let v = w0 * t.uv[0][1] + w1 * t.uv[1][1] + w2 * t.uv[2][1];
                    let tx = ((u.rem_euclid(1.0)) * img.width as f32) as usize % img.width as usize;
                    let ty = ((v.rem_euclid(1.0)) * img.height as f32) as usize % img.height as usize;
                    let c = &img.rgba[(ty * img.width as usize + tx) * 4..][..4];
                    if c[3] < 8 && std::env::var_os("SIGNET_ALPHATEST").is_some() {
                        continue;
                    }
                    rgb = [c[0] as f32, c[1] as f32, c[2] as f32];
                }
                depth[i] = z;
                color[i] = [
                    (rgb[0] * t.shade) as u8,
                    (rgb[1] * t.shade) as u8,
                    (rgb[2] * t.shade) as u8,
                    255,
                ];
            }
        }
    }
    let rgba: Vec<u8> = color.into_iter().flatten().collect();
    std::fs::write(&out, png::encode_rgba(size as u32, size as u32, &rgba)).expect("write png");
    println!(
        "placements {}  meshes {}  textures {}  triangles {}  sub-meshes textured {tex_hit} untextured {tex_miss}",
        layout.len(),
        meshes.len(),
        images.len(),
        tris.len()
    );
    println!("world x {:.0}..{:.0}  y {:.0}..{:.0}  z {:.0}..{:.0}  -> {}", lo[0], hi[0], lo[1], hi[1], lo[2], hi[2], out.display());
}
