//! Finds XISTRING tables in the install: id, entry count, and a few sample entries.
//! Usage: signet-dat-xistrings [install-root] [first-id] [last-id] [probe-index ...]
use signet_dat::zones::parse_string_table;
use signet_dat::{Install, find_install_root};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let root = a.first().filter(|s| s.parse::<u32>().is_err()).map(Into::into).or_else(find_install_root).expect("no install");
    let nums: Vec<u32> = a.iter().filter_map(|s| s.parse().ok()).collect();
    let install = Install::open(&root).expect("open");
    let (lo, hi) = (nums.first().copied().unwrap_or(0), nums.get(1).copied().unwrap_or(install.id_count()));
    let probes: Vec<usize> = if nums.len() > 2 { nums[2..].iter().map(|&n| n as usize).collect() } else { vec![0, 100, 230, 256, 300] };
    for id in lo..hi.min(install.id_count()) {
        let Ok(b) = install.read(id) else { continue };
        if b.get(..8) != Some(b"XISTRING") {
            continue;
        }
        let Some(t) = parse_string_table(&b) else { println!("id {id}: XISTRING (unparsed)"); continue };
        let s: Vec<String> = probes.iter().map(|&i| format!("[{i}]={}", t.get(i).map_or("-", |s| s.as_str()))).collect();
        println!("id {id}: {} entries  {}", t.len(), s.join("  "));
    }
}
