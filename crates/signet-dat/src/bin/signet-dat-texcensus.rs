//! Decodes every texture chunk in the install and counts formats, sizes and failures.
//! Usage: signet-dat-texcensus [install-root] [--limit N]

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use signet_dat::{Install, chunks, find_install_root, texture};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let limit = args
        .iter()
        .position(|a| a == "--limit")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<u32>().ok());
    let root = args
        .iter()
        .find(|a| !a.starts_with("--") && a.parse::<u32>().is_err())
        .map(PathBuf::from)
        .or_else(find_install_root)
        .expect("no install root");
    let install = Install::open(&root).expect("open install");
    let start = Instant::now();
    let ids = limit.unwrap_or(install.id_count()).min(install.id_count());

    let mut seen = HashSet::new();
    let mut formats: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut sizes: BTreeMap<(u32, u32), u64> = BTreeMap::new();
    let mut errors: BTreeMap<String, u64> = BTreeMap::new();
    let mut examples: Vec<String> = Vec::new();
    let (mut files, mut ok, mut failed) = (0u64, 0u64, 0u64);

    for id in 0..ids {
        let Some(loc) = install.locate(id) else { continue };
        if !seen.insert(loc) {
            continue;
        }
        let Ok(data) = std::fs::read(install.path(loc)) else { continue };
        files += 1;
        for c in chunks(&data).filter_map(Result::ok).filter(|c| c.kind == texture::KIND) {
            match texture::decode(c.data) {
                Ok((h, img)) => {
                    ok += 1;
                    let e = formats.entry(format!("{:?} (flag 0x{:02x})", h.format, h.flag)).or_default();
                    e.0 += 1;
                    e.1 += img.rgba.len() as u64;
                    *sizes.entry((h.width, h.height)).or_default() += 1;
                }
                Err(e) => {
                    failed += 1;
                    let key = match &e {
                        texture::TextureError::Truncated { .. } => "truncated".to_string(),
                        other => other.to_string(),
                    };
                    let n = errors.entry(key).or_default();
                    *n += 1;
                    if examples.len() < 15 {
                        examples.push(format!(
                            "id {id} {:?} at {}: {e} (payload {} bytes, flag 0x{:02x})",
                            loc.relative_path(),
                            c.offset,
                            c.data.len(),
                            c.data.first().copied().unwrap_or(0)
                        ));
                    }
                }
            }
        }
    }

    println!("install: {}", root.display());
    println!("files with ids < {ids}: {files}");
    println!("textures decoded: {ok}  failed: {failed}");
    println!("formats (count, decoded RGBA bytes):");
    for (k, (n, b)) in &formats {
        println!("  {k}: {n} {b}");
    }
    println!("errors:");
    for (k, n) in &errors {
        println!("  {k}: {n}");
    }
    for e in &examples {
        println!("  example: {e}");
    }
    let mut by_count: Vec<_> = sizes.into_iter().collect();
    by_count.sort_by(|a, b| b.1.cmp(&a.1));
    println!("top sizes:");
    for ((w, h), n) in by_count.iter().take(12) {
        println!("  {w}x{h}: {n}");
    }
    println!("elapsed: {:.1}s", start.elapsed().as_secs_f64());
}
