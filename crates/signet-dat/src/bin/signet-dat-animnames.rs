//! Lists the animation names (and track counts) of DATs by file id.
//! Usage: signet-dat-animnames <first-id> [<last-id>] [--root <install>]
use signet_dat::{Install, anim, chunks, find_install_root};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let root = a.iter().position(|x| x == "--root").and_then(|i| a.get(i + 1)).map(Into::into).or_else(find_install_root).expect("no install root");
    let nums: Vec<u32> = a.iter().filter_map(|s| s.parse().ok()).collect();
    let (lo, hi) = (nums[0], *nums.get(1).unwrap_or(&nums[0]));
    let install = Install::open(&root).expect("open install");
    for id in lo..=hi {
        let Ok(dat) = install.read(id) else { continue };
        let mut names = Vec::new();
        for c in chunks(&dat).filter_map(Result::ok) {
            if c.kind == anim::KIND {
                let tracks = anim::parse(c.data).map(|x| x.tracks.len()).unwrap_or(0);
                names.push(format!("{}:{tracks}", c.name_str().trim_end_matches('.')));
            }
        }
        if !names.is_empty() {
            println!("{id}: {}", names.join(" "));
        }
    }
}
