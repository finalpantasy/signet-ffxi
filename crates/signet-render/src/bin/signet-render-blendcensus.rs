//! Census of sub-mesh blend values against texture alpha content across zone DATs.
//! Usage: signet-render-blendcensus [--from N] [--to N] [--root DIR]
use std::collections::{BTreeMap, HashMap, HashSet};

use signet_dat::crypt::{self, ZoneKeys};
use signet_dat::{Install, chunks, find_install_root, mmb, texture};

#[derive(Default)]
struct Row {
    subs: u64,
    tris: u64,
    // alpha class: opaque / binary / graded / missing-texture
    class: [u64; 6],
    fmt: BTreeMap<String, u64>,
    examples: Vec<String>,
    names: BTreeMap<(usize, String), u64>,
    valpha: [u64; 8],
    vtrans: u64,
    under: [u64; 6],
    under_names: BTreeMap<String, u64>,
    plain: [u64; 6],
    vtrans_names: BTreeMap<String, u64>,
}

const CLASSES: [&str; 6] = ["opaque", "uniform", "binary", "soft", "heavy", "notex"];

/// 0 opaque (>=99.5% texels a>=240), 1 uniform translucent, 2 binary cutout (<=5% mid),
/// 3 soft (mid>5%, <5% clear), 4 heavy (mid>5%, >=5% clear).
fn classify(rgba: &[u8]) -> usize {
    let n = (rgba.len() / 4).max(1) as f64;
    let (mut zero, mut full, mut mid) = (0u64, 0u64, 0u64);
    let (mut lo, mut hi) = (255u8, 0u8);
    for p in rgba.chunks_exact(4) {
        lo = lo.min(p[3]);
        hi = hi.max(p[3]);
        match p[3] {
            0..=15 => zero += 1,
            240..=255 => full += 1,
            _ => mid += 1,
        }
    }
    let (z, f, m) = (zero as f64 / n, full as f64 / n, mid as f64 / n);
    if f >= 0.995 {
        0
    } else if hi - lo <= 8 {
        1
    } else if m <= 0.05 && z >= 0.005 {
        2
    } else if m > 0.05 && z < 0.05 {
        3
    } else if m > 0.05 {
        4
    } else {
        0
    }
}

fn dump(data: &[u8], _keys: &ZoneKeys, meshes: &[mmb::Mesh], placed: &HashSet<String>) {
    let mut imgs: HashMap<String, (Vec<u32>, String)> = HashMap::new();
    for c in chunks(data).filter_map(Result::ok).filter(|c| c.kind == texture::KIND) {
        if let Ok((h, img)) = texture::decode(c.data) {
            let mut hist = vec![0u32; 8];
            for p in img.rgba.chunks_exact(4) {
                hist[(p[3] >> 5) as usize] += 1;
            }
            imgs.insert(h.name.clone(), (hist, format!("{:?} flag{:02x} {}x{}", h.format, h.flag, h.width, h.height)));
        }
    }
    // per texture: blends, count, vertex alpha histogram
    let mut use_: BTreeMap<String, (BTreeMap<u16, u32>, [u32; 8])> = BTreeMap::new();
    for m in meshes.iter().filter(|m| placed.contains(&m.name)) {
        for s in &m.submeshes {
            let name = s.texture.get(8..).unwrap_or(&s.texture).trim().to_string();
            let e = use_.entry(name).or_default();
            *e.0.entry(s.blend).or_default() += 1;
            for v in &s.vertices {
                e.1[(v.color >> 29) as usize] += 1;
            }
        }
    }
    for (n, (b, va)) in &use_ {
        let (h, f) = imgs.get(n).cloned().unwrap_or((vec![], "missing".into()));
        println!("{n:<10} blends {b:x?} texalpha/32 {h:?} {f} vertalpha/32 {va:?}");
    }
}

// PS2 alpha range: 0x80 is opaque; DXT3's 4-bit dithering leaves "opaque" at 0x77/0x88.
use signet_dat::texture::ps2_alpha;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let get = |k: &str| a.iter().position(|x| x == k).and_then(|i| a.get(i + 1)).cloned();
    let from: u32 = get("--from").and_then(|v| v.parse().ok()).unwrap_or(100);
    let to: u32 = get("--to").and_then(|v| v.parse().ok()).unwrap_or(1500);
    let root = get("--root").map(Into::into).or_else(find_install_root).expect("no install root");
    if let (Some(z), Some(names), Some(out)) = (get("--png"), get("--names"), get("--out")) {
        let install = Install::open(&get("--root").map(Into::into).or_else(find_install_root).expect("root")).unwrap();
        // A number is a file id; anything else is a path to a DAT.
        let data = match z.parse::<u32>() {
            Ok(id) => install.read(id).expect("read"),
            Err(_) => std::fs::read(&z).expect("read DAT"),
        };
        let mut tiles: Vec<(u32, u32, Vec<u8>)> = Vec::new();
        for n in names.split(',') {
            for c in chunks(&data).filter_map(Result::ok).filter(|c| c.kind == texture::KIND) {
                if let Ok((h, img)) = texture::decode(c.data) {
                    if h.name.trim() == n {
                        // left: color over magenta; right: alpha as gray
                        let (w, hh) = (img.width as usize, img.height as usize);
                        let mut t = vec![255u8; w * 2 * hh * 4];
                        for y in 0..hh {
                            for x in 0..w {
                                let p = &img.rgba[(y * w + x) * 4..][..4];
                                let a = p[3] as u32;
                                let mg = [255u32, 0, 255];
                                for k in 0..3 {
                                    t[(y * 2 * w + x) * 4 + k] = ((p[k] as u32 * a + mg[k] * (255 - a)) / 255) as u8;
                                    t[(y * 2 * w + w + x) * 4 + k] = p[3];
                                }
                            }
                        }
                        tiles.push((img.width * 2, img.height, t));
                        break;
                    }
                }
            }
        }
        let tw = tiles.iter().map(|t| t.0).max().unwrap_or(1);
        let th: u32 = tiles.iter().map(|t| t.1).sum();
        let mut sheet = vec![0u8; (tw * th * 4) as usize];
        let mut y0 = 0;
        for (w, h, t) in &tiles {
            for y in 0..*h {
                let d = ((y0 + y) * tw * 4) as usize;
                sheet[d..d + (*w * 4) as usize].copy_from_slice(&t[(y * w * 4) as usize..][..(*w * 4) as usize]);
            }
            y0 += h;
        }
        std::fs::write(out, signet_dat::png::encode_rgba(tw, th, &sheet)).unwrap();
        return;
    }
    if let (Some(z), Some(names)) = (get("--hdr").and_then(|v| v.parse::<u32>().ok()), get("--names")) {
        let install = Install::open(&get("--root").map(Into::into).or_else(find_install_root).expect("root")).unwrap();
        let data = install.read(z).expect("read");
        for c in chunks(&data).filter_map(Result::ok).filter(|c| c.kind == texture::KIND) {
            if let Ok((h, img)) = texture::decode(c.data) {
                if names.split(',').any(|n| n == h.name.trim()) || names == "*" {
                    let cls = classify(&img.rgba);
                    println!("{:<9} {:?} {}x{} cls {} hdr {:02x?}", h.name, h.format, h.width, h.height, cls, &c.data[..c.data.len().min(80)]);
                }
            }
        }
        return;
    }
    let keys = ZoneKeys::from_install(&root).expect("zone keys");
    let install = Install::open(&root).expect("open install");
    let mut seen = HashSet::new();
    let mut rows: BTreeMap<u16, Row> = BTreeMap::new();
    let mut zones = 0;
    for id in from..to.min(install.id_count()) {
        let Some(loc) = install.locate(id) else { continue };
        if !seen.insert(loc) {
            continue;
        }
        let Ok(data) = std::fs::read(install.path(loc)) else { continue };
        let mut tex: HashMap<String, (usize, String)> = HashMap::new();
        let mut meshes = Vec::new();
        let mut placed: HashSet<String> = HashSet::new();
        for c in chunks(&data).filter_map(Result::ok) {
            match c.kind {
                texture::KIND => {
                    if let Ok((h, img)) = texture::decode(c.data) {
                        if img.rgba.len() >= (img.width * img.height * 4) as usize && img.width > 0 {
                            tex.insert(h.name.clone(), (classify(&img.rgba), format!("{:?}", h.format)));
                        }
                    }
                }
                crypt::MMB_KIND => {
                    let mut p = c.data.to_vec();
                    crypt::decode_mmb(&mut p, &keys);
                    if let Ok(m) = mmb::parse(&p) {
                        meshes.push(m);
                    }
                }
                crypt::MZB_KIND => {
                    let mut p = c.data.to_vec();
                    crypt::decode_mzb(&mut p, &keys);
                    placed.extend(crypt::placements(&p).into_iter().map(|x| x.mesh));
                }
                _ => {}
            }
        }
        if meshes.is_empty() || placed.is_empty() {
            continue;
        }
        zones += 1;
        if let Some(z) = get("--dump").and_then(|v| v.parse::<u32>().ok()) {
            if z == id {
                dump(&data, &keys, &meshes, &placed);
            }
        }
        for m in meshes.iter().filter(|m| placed.contains(&m.name)) {
            for s in &m.submeshes {
                let r = rows.entry(s.blend).or_default();
                r.subs += 1;
                let va_min = s.vertices.iter().map(|v| ps2_alpha((v.color >> 24) as u8)).min().unwrap_or(255);
                if va_min < 0xF0 {
                    r.vtrans += 1;
                    *r.vtrans_names.entry(s.texture.get(8..).unwrap_or("").trim().to_string()).or_default() += 1;
                }
                for v in &s.vertices {
                    r.valpha[(v.color >> 29) as usize] += 1;
                }
                r.tris += s.indices.len() as u64 / 3;
                let name = s.texture.get(8..).unwrap_or(&s.texture).trim().to_string();
                if get("--find").is_some_and(|f| f.split(',').any(|x| x == name)) {
                    println!("find {name} zone {id} blend 0x{:04x}", s.blend);
                }
                let (cls, fmt) = tex.get(&name).cloned().unwrap_or((5, "-".into()));
                r.class[cls] += 1;
                if m.name.starts_with('_') {
                    r.under[cls] += 1;
                    *r.under_names.entry(name.clone()).or_default() += 1;
                } else {
                    r.plain[cls] += 1;
                }
                *r.names.entry((cls, name.clone())).or_default() += 1;
                *r.fmt.entry(fmt).or_default() += 1;
                if r.examples.len() < 8 && cls != 0 && cls != 5 && !r.examples.iter().any(|e| e.starts_with(&name)) {
                    r.examples.push(format!("{name}(zone {id},c{cls})"));
                }
            }
        }
    }
    println!("zones with geometry: {zones}");
    for (b, r) in &rows {
        for cls in 0..5 {
            let mut v: Vec<_> = r.names.iter().filter(|((c, _), _)| *c == cls).map(|((_, n), k)| (*k, n.clone())).collect();
            v.sort_by(|a, b| b.cmp(a));
            let top: Vec<String> = v.iter().take(10).map(|(k, n)| format!("{n}x{k}")).collect();
            println!("blend 0x{b:04x} {:<8}: {}", CLASSES[cls], top.join(" "));
        }
    }
    for (b, r) in &rows {
        let mut v: Vec<_> = r.vtrans_names.iter().map(|(n, k)| (*k, n.clone())).collect();
        v.sort_by(|a, b| b.cmp(a));
        let top: Vec<String> = v.iter().take(12).map(|(k, n)| format!("{n}x{k}")).collect();
        println!("blend 0x{b:04x} vertex alpha/32 histogram {:?}  submeshes with min vertex alpha < 0xF0: {} of {}  {}", r.valpha, r.vtrans, r.subs, top.join(" "));
    }
    for (b, r) in &rows {
        let mut v: Vec<_> = r.under_names.iter().map(|(n, k)| (*k, n.clone())).collect();
        v.sort_by(|a, b| b.cmp(a));
        let top: Vec<String> = v.iter().take(8).map(|(k, n)| format!("{n}x{k}")).collect();
        println!("blend 0x{b:04x} mesh name starts with '_': {:?} {}   other meshes: {:?}", r.under, top.join(" "), r.plain);
    }
    println!("blend  subs  tris  | {:?} | formats | examples", CLASSES);
    for (b, r) in &rows {
        println!(
            "0x{b:04x} {:>6} {:>8} | {:?} | {:?} | {}",
            r.subs, r.tris, r.class, r.fmt, r.examples.join(" ")
        );
    }
}
