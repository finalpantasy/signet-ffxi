//! Prints every environment key chunk (kind 0x2f) of one DAT as hex plus the light colors.
//! Usage: signet-dat-envdump <file-id> [--root <install>]
use signet_dat::{Install, chunks, find_install_root};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let root = a.iter().position(|x| x == "--root").and_then(|i| a.get(i + 1)).map(Into::into).or_else(find_install_root).expect("no install root");
    let id: u32 = a[0].parse().expect("file id");
    let dat = Install::open(&root).expect("open install").read(id).expect("read id");
    for c in chunks(&dat).filter_map(Result::ok) {
        if c.kind != 0x2f {
            continue;
        }
        println!("key {}", c.name_str());
        for (i, row) in c.data.chunks(16).enumerate() {
            let hex: Vec<String> = row.iter().map(|b| format!("{b:02x}")).collect();
            println!("  {:02x}: {}", i * 16, hex.join(" "));
        }
    }
}
