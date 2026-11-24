//! Walks every file id in a local install: checks the tables, reads each DAT once,
//! and confirms its chunk stream ends exactly at end of file. Prints a summary.
//! Usage: signet-dat-scan [install-root] [--limit N]

use std::collections::{BTreeMap, HashSet};
use std::time::Instant;

use signet_dat::{Install, chunks, find_install_root};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let limit = args
        .iter()
        .position(|a| a == "--limit")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<u32>().ok());
    let root = args
        .iter()
        .find(|a| !a.starts_with("--") && a.parse::<u32>().is_err())
        .map(Into::into)
        .or_else(find_install_root)
        .expect("no install: pass a path or set SIGNET_FFXI_ROOT");
    let install = Install::open(&root).expect("open install");
    let started = Instant::now();
    let ids = limit.unwrap_or(install.id_count()).min(install.id_count());

    let (mut claimed, mut multi, mut missing, mut read_ok, mut bad, mut bytes) = (0u32, 0u32, 0u32, 0u32, 0u32, 0u64);
    let mut kinds: BTreeMap<u8, (u32, u64)> = BTreeMap::new();
    let mut seen_paths = HashSet::new();
    let mut shared_paths = 0u32;
    let mut bad_examples = Vec::new();
    let (mut padded, mut not_chunked) = (0u32, 0u32);
    let mut not_chunked_examples = Vec::new();

    for id in 0..ids {
        let claims: Vec<_> = install.claims(id).collect();
        if claims.is_empty() {
            continue;
        }
        claimed += 1;
        if claims.len() > 1 {
            multi += 1;
        }
        let loc = *claims.last().unwrap();
        if !seen_paths.insert(loc) {
            shared_paths += 1;
            continue;
        }
        let data = match std::fs::read(install.path(loc)) {
            Ok(d) => d,
            Err(_) => {
                missing += 1;
                continue;
            }
        };
        read_ok += 1;
        bytes += data.len() as u64;
        let mut ok = true;
        let mut local: Vec<(u8, usize)> = Vec::new();
        let mut it = chunks(&data);
        for c in it.by_ref() {
            match c {
                Ok(c) => local.push((c.kind, c.data.len())),
                Err(e) => {
                    ok = false;
                    if bad_examples.len() < 8 {
                        bad_examples.push(format!("id {id} {:?}: {e}", loc.relative_path()));
                    }
                    break;
                }
            }
        }
        if ok && it.stream_end() != Some(data.len()) {
            padded += 1;
        }
        if !ok && local.is_empty() {
            not_chunked += 1;
            ok = true;
            if not_chunked_examples.len() < 6 {
                not_chunked_examples.push(format!("{:?} ({} bytes)", loc.relative_path(), data.len()));
            }
        }
        if ok {
            for (k, n) in local {
                let e = kinds.entry(k).or_default();
                e.0 += 1;
                e.1 += n as u64;
            }
        } else {
            bad += 1;
        }
    }

    println!("install: {}", root.display());
    println!("roms: {:?}  ids: {}", install.roms().collect::<Vec<_>>(), install.id_count());
    println!("scanned ids: {ids}  claimed: {claimed}  claimed by >1 rom: {multi}  ids sharing a path: {shared_paths}");
    println!("files read: {read_ok}  missing on disk: {missing}  bytes: {}", bytes);
    println!(
        "chunk files: {} (ending at EOF: {}, ending at a zero header before padding: {padded})",
        read_ok - bad - not_chunked,
        read_ok - bad - not_chunked - padded
    );
    println!("other formats (first header is not a chunk): {not_chunked}  broken mid-stream: {bad}");
    for e in &not_chunked_examples {
        println!("  other: {e}");
    }
    for e in &bad_examples {
        println!("  bad: {e}");
    }
    println!("chunk kinds (kind: count, payload bytes):");
    for (k, (n, b)) in &kinds {
        println!("  0x{k:02x}: {n} {b}");
    }
    println!("elapsed: {:.1}s", started.elapsed().as_secs_f64());
}
