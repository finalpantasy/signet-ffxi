//! Loads a zone DAT into world-space geometry grouped by texture.
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use signet_dat::crypt::{self, ZoneKeys};
use signet_dat::texture::{self, Image};
use signet_dat::{chunks, mmb};

use crate::mips::{self, TexClass};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub color: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pass {
    Opaque,
    Cutout,
    Blend,
}

#[derive(Clone)]
pub struct Draw {
    /// 0 is the fallback gray texture; n > 0 is `slots[n - 1]`.
    pub texture: usize,
    pub first_index: u32,
    pub index_count: u32,
    pub pass: Pass,
    /// World-space center, used to sort blended draws back to front.
    pub center: [f32; 3],
}

pub struct Scene {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub draws: Vec<Draw>,
    pub images: Vec<Image>,
    /// Texture slots: (index into `images`, mip class for that use). `Draw::texture` n > 0 is
    /// `slots[n - 1]`. One image used by both opaque and alpha-tested meshes gets two slots, so
    /// the transparent-texel color fill never touches the opaque use.
    pub slots: Vec<(usize, TexClass)>,
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    pub stats: String,
    /// Zone collision from the MZB, when it decodes.
    pub collision: Option<signet_dat::collision::Collision>,
    /// Weather and time-of-day lighting keys from the zone DAT, when present.
    pub environment: Option<signet_dat::environment::Environment>,
}

/// PS2 alpha (0x80 = opaque) to 8-bit; used for vertex alpha (texture alpha is converted in the DAT reader).
pub use signet_dat::texture::ps2_alpha;

fn rotate(v: [f32; 3], r: [f32; 3]) -> [f32; 3] {
    let (sx, cx) = r[0].sin_cos();
    let (sy, cy) = r[1].sin_cos();
    let (sz, cz) = r[2].sin_cos();
    let v = [v[0], v[1] * cx - v[2] * sx, v[1] * sx + v[2] * cx];
    let v = [v[0] * cy + v[2] * sy, v[1], -v[0] * sy + v[2] * cy];
    [v[0] * cz - v[1] * sz, v[0] * sz + v[1] * cz, v[2]]
}

pub fn load(dat_path: &Path, root: &Path) -> Result<Scene, String> {
    let dat = std::fs::read(dat_path).map_err(|e| format!("read {}: {e}", dat_path.display()))?;
    let keys = ZoneKeys::from_install(root).map_err(|e| format!("zone keys: {e:?}"))?;
    let mut images: Vec<Image> = Vec::new();
    let mut tex_by_name: HashMap<String, usize> = HashMap::new();
    let mut meshes: HashMap<String, mmb::Mesh> = HashMap::new();
    let mut layout = Vec::new();
    let mut collision = None;
    for c in chunks(&dat).filter_map(Result::ok) {
        match c.kind {
            texture::KIND => {
                if let Ok((h, img)) = texture::decode(c.data) {
                    if img.width == 0 || img.height == 0 || img.rgba.len() < (img.width * img.height * 4) as usize {
                        continue;
                    }
                    let slot = images.len() + 1;
                    tex_by_name.insert(format!("{:<8}{:<8}", h.group, h.name).trim_end().to_string(), slot);
                    tex_by_name.entry(h.name.clone()).or_insert(slot);
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
                collision = signet_dat::collision::Collision::parse(&p).ok();
            }
            _ => {}
        }
    }

    // SIGNET_ALPHA=off draws everything opaque (before/after comparisons).
    let alpha_off = std::env::var("SIGNET_ALPHA").is_ok_and(|v| v == "off");
    let classes: Vec<TexClass> = images.iter().map(|i| mips::classify(&i.rgba)).collect();
    let mut slots: Vec<(usize, TexClass)> = Vec::new();
    let mut slot_map: HashMap<(usize, TexClass), usize> = HashMap::new();
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut by_tex: BTreeMap<(u8, usize), Vec<u32>> = BTreeMap::new();
    let mut blended: Vec<(usize, Vec<u32>, [f32; 3])> = Vec::new();
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    let (mut skip_mesh, mut skip_empty, mut skip_big, mut untextured, mut submeshes) = (0u32, 0u32, 0u32, 0u32, 0u32);
    let mut pass_count = [0u32; 3];
    let dump = std::env::var_os("SIGNET_DUMP_SUBMESHES").is_some();
    let mut dump_tab: BTreeMap<(String, String, u16), (u32, u32, u32)> = BTreeMap::new();
    for pl in &layout {
        let Some(m) = meshes.get(&pl.mesh) else {
            skip_mesh += 1;
            continue;
        };
        for s in &m.submeshes {
            if s.vertices.is_empty() || s.indices.len() < 3 {
                skip_empty += 1;
                continue;
            }
            if vertices.len() + s.vertices.len() > u32::MAX as usize / 2 {
                skip_big += 1;
                continue;
            }
            let tex = tex_by_name.get(&s.texture).copied().unwrap_or_else(|| {
                untextured += 1;
                0
            });
            submeshes += 1;
            // Alpha rule (census of 539 zones): texture alpha picks cutout vs opaque; the 0x8000
            // blend bit marks meshes whose vertex alpha fades them (shore and ground overlays), so
            // those blend when the vertex alpha is actually below opaque.
            let class = if tex == 0 { TexClass::Opaque } else { classes[tex - 1] };
            let va_min = s.vertices.iter().map(|v| ps2_alpha((v.color >> 24) as u8)).min().unwrap_or(255);
            let pass = if alpha_off {
                Pass::Opaque
            } else if s.blend & 0x8000 != 0 {
                // Fading meshes: texture alpha times vertex alpha, blended.
                if class == TexClass::Opaque && va_min >= 0xF0 { Pass::Opaque } else { Pass::Blend }
            } else if m.name.starts_with('_') && class != TexClass::Opaque {
                // Foliage-style meshes: alpha-tested.
                Pass::Cutout
            } else {
                Pass::Opaque
            };
            let use_class = match pass {
                Pass::Opaque => TexClass::Opaque,
                Pass::Cutout => TexClass::Cutout,
                Pass::Blend => TexClass::Translucent,
            };
            let tex = if tex == 0 {
                0
            } else {
                *slot_map.entry((tex - 1, use_class)).or_insert_with(|| {
                    slots.push((tex - 1, use_class));
                    slots.len()
                })
            };
            pass_count[pass as usize] += 1;
            if dump {
                if !dump_tab.keys().any(|k| k.0 == m.name) {
                    eprintln!("POS {} {:?}", m.name, pl.position);
                }
                let e = dump_tab.entry((m.name.clone(), s.texture.clone(), s.blend)).or_default();
                e.0 += 1;
                e.1 = pass as u32;
                e.2 = if tex == 0 { 9 } else { slots[tex - 1].1 as u32 };
            }
            let base = vertices.len() as u32;
            let mut sum = [0f32; 3];
            for v in &s.vertices {
                let sc = [v.position[0] * pl.scale[0], v.position[1] * pl.scale[1], v.position[2] * pl.scale[2]];
                let r = rotate(sc, pl.rotation);
                let pos = [r[0] + pl.position[0], r[1] + pl.position[1], r[2] + pl.position[2]];
                for k in 0..3 {
                    lo[k] = lo[k].min(pos[k]);
                    hi[k] = hi[k].max(pos[k]);
                    sum[k] += pos[k];
                }
                vertices.push(Vertex { pos, normal: rotate(v.normal, pl.rotation), uv: v.uv, color: v.color });
            }
            let idx = s.indices.iter().map(|i| base + i);
            if pass == Pass::Blend {
                let n = s.vertices.len() as f32;
                blended.push((tex, idx.collect(), [sum[0] / n, sum[1] / n, sum[2] / n]));
            } else {
                by_tex.entry((pass as u8, tex)).or_default().extend(idx);
            }
        }
    }
    if dump {
        let n = vertices.len() as f64;
        let mut hist = [[0u64; 8]; 3];
        let mut mean = [0f64; 4];
        for v in &vertices {
            for k in 0..4 {
                let b = (v.color >> (8 * k)) & 0xff;
                mean[k] += b as f64;
                if k < 3 { hist[k][(b / 32) as usize] += 1; }
            }
        }
        eprintln!("VCOLOR mean b,g,r,a = {:.1} {:.1} {:.1} {:.1} (0x80 = neutral); hist(r) by 32s {:?}", mean[0] / n, mean[1] / n, mean[2] / n, mean[3] / n, hist[2]);
        for ((mesh, tex, blend), (n, pass, class)) in &dump_tab {
            eprintln!("SUBMESH mesh={mesh:<10} tex={tex:<10} blend={blend:#06x} n={n} pass={pass} slotclass={class}");
        }
    }
    if vertices.is_empty() {
        return Err("zone has no drawable geometry".into());
    }
    let mut indices = Vec::new();
    let mut draws = Vec::new();
    for ((pass, texture), list) in by_tex {
        let pass = if pass == Pass::Opaque as u8 { Pass::Opaque } else { Pass::Cutout };
        draws.push(Draw { texture, first_index: indices.len() as u32, index_count: list.len() as u32, pass, center: [0.0; 3] });
        indices.extend(list);
    }
    for (texture, list, center) in blended {
        draws.push(Draw { texture, first_index: indices.len() as u32, index_count: list.len() as u32, pass: Pass::Blend, center });
        indices.extend(list);
    }
    let stats = format!(
        "placements {}  meshes {}  texture slots {}  sub-meshes drawn {submeshes} (opaque {}, cutout {}, blend {}; untextured/gray {untextured})  \
         skipped: placement without mesh {skip_mesh}, empty sub-mesh {skip_empty}, over vertex budget {skip_big}  \
         vertices {}  triangles {}  draws {}",
        layout.len(),
        meshes.len(),
        slots.len(),
        pass_count[0],
        pass_count[1],
        pass_count[2],
        vertices.len(),
        indices.len() / 3,
        draws.len()
    );
    let environment = signet_dat::environment::Environment::parse(&dat);
    Ok(Scene { vertices, indices, draws, images, slots, lo, hi, stats, collision, environment })
}
