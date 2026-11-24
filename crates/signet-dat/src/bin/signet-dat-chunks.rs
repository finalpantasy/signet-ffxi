//! Lists the chunks of a zone DAT (by zone id) grouped by kind, optionally hex-dumping one kind.
//! Usage: signet-dat-chunks <zone-id> [--kind 0xNN] [--bytes N] [--root <install>]
use std::collections::BTreeMap;

use signet_dat::zones::model_file_id;
use signet_dat::{Install, chunks, find_install_root};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let get = |k: &str| a.iter().position(|x| x == k).and_then(|i| a.get(i + 1)).cloned();
    let root = get("--root").map(Into::into).or_else(find_install_root).expect("no install");
    let id: u16 = a.first().and_then(|s| s.parse().ok()).expect("zone id");
    let kind = get("--kind").map(|k| u8::from_str_radix(k.trim_start_matches("0x"), 16).expect("hex kind"));
    let name_filter = get("--name");
    let nbytes: usize = get("--bytes").and_then(|v| v.parse().ok()).unwrap_or(96);
    let install = Install::open(&root).expect("open");
    let dat = install.read(model_file_id(id)).expect("read zone dat");
    if a.iter().any(|x| x == "--tree") {
        let mut depth = 0usize;
        for c in chunks(&dat).filter_map(Result::ok) {
            if c.kind == 0 {
                depth = depth.saturating_sub(1);
                continue;
            }
            if a.iter().any(|x| x == "--env") && c.kind == 0x2f && c.data.len() == 176 {
                let f = |o: usize| f32::from_le_bytes(c.data[o..o + 4].try_into().unwrap());
                let col = |o: usize| format!("{:02x}{:02x}{:02x}/{:02x}", c.data[o], c.data[o + 1], c.data[o + 2], c.data[o + 3]);
                println!(
                    "{}{} w={:?}  A[{} {} {} {}] {:.1} {:.2} {:.2} {:.2} | B[{} {} {} {}] {:.1} {:.2} {:.2} {:.2} | C[{}] {:.1} {:.1} {:.1} {:08x} {:.1} {:.1} | G {} {} {} {} {} {} {} {}",
                    "  ".repeat(depth), c.name_str(), c.data[0], col(12), col(16), col(20), col(24), f(28), f(32), f(36), f(40),
                    col(44), col(48), col(52), col(56), f(60), f(64), f(68), f(72), col(76), f(80), f(84), f(88),
                    u32::from_le_bytes(c.data[92..96].try_into().unwrap()), f(96), f(104),
                    col(108), col(112), col(116), col(120), col(124), col(128), col(132), col(136)
                );
                continue;
            }
            if !matches!(c.kind, 0x01 | 0x2f | 0x21 | 0x19) || (a.iter().any(|x| x == "--env") && c.kind != 0x01) {
                continue;
            }
            println!("{}{:?} 0x{:02x} len {}", "  ".repeat(depth), c.name_str(), c.kind, c.data.len());
            if c.kind == 0x01 {
                depth += 1;
            }
        }
        return;
    }
    let mut by: BTreeMap<u8, Vec<(String, usize)>> = BTreeMap::new();
    for c in chunks(&dat).filter_map(Result::ok) {
        by.entry(c.kind).or_default().push((c.name_str(), c.data.len()));
        if Some(c.kind) == kind && name_filter.as_ref().is_none_or(|n| c.name_str().trim_end_matches('.') == n) {
            println!("kind 0x{:02x} name {:?} len {}", c.kind, c.name_str(), c.data.len());
            for (i, row) in c.data[..c.data.len().min(nbytes)].chunks(16).enumerate() {
                println!("  {:04x}: {}", i * 16, row.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "));
            }
        }
    }
    println!("zone {id}: {} bytes", dat.len());
    for (k, v) in &by {
        let names: Vec<String> = v.iter().take(if a.iter().any(|x| x == "--all") { usize::MAX } else { 4 }).map(|(n, l)| format!("{n}:{l}")).collect();
        println!("kind 0x{k:02x}: {} chunks, {} bytes  e.g. {}", v.len(), v.iter().map(|x| x.1).sum::<usize>(), names.join(" "));
    }
}
