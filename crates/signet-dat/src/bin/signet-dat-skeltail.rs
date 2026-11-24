//! Prints a skeleton's bone count, the trailing block after the bones and the world bind pose of the last bones.
//! Usage: signet-dat-skeltail <file-id> [--root <install>]
use signet_dat::{Install, chunks, find_install_root, skeleton};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let root = a.iter().position(|x| x == "--root").and_then(|i| a.get(i + 1)).map(Into::into).or_else(find_install_root).expect("no install root");
    let id: u32 = a[0].parse().unwrap();
    let dat = Install::open(&root).expect("open install").read(id).expect("read");
    for c in chunks(&dat).filter_map(Result::ok) {
        if c.kind != skeleton::KIND {
            continue;
        }
        let p = c.data;
        let sk = skeleton::parse(p).unwrap();
        let n = sk.bones.len();
        let tail = 4 + n * 30;
        println!("skeleton {:?}: {n} bones, payload {} bytes, tail starts at {tail} ({} bytes)", c.name_str(), p.len(), p.len() - tail);
        let world = sk.bind_world();
        for i in (n.saturating_sub(34))..n {
            let w = world[i];
            println!("  bone {i} parent {:?} leaf {} world t ({:.3}, {:.3}, {:.3}) q ({:.3} {:.3} {:.3} {:.3})", sk.bones[i].parent, sk.bones[i].leaf, w.translation[0], w.translation[1], w.translation[2], w.rotation[0], w.rotation[1], w.rotation[2], w.rotation[3]);
        }
        let t = &p[tail..];
        println!("  tail head: {:02x?}", &t[..t.len().min(48)]);
        // doc: u16 count at offset 4 of the block, then 26-byte records (u16 bone + 6 floats)
        if t.len() >= 6 {
            let cnt = u16::from_le_bytes([t[0], t[1]]) as usize;
            println!("  ref count {cnt}");
            for k in 0..cnt.min(128) {
                let o = 4 + k * 26;
                if o + 26 > t.len() { break; }
                let f = |q: usize| f32::from_le_bytes([t[o + 2 + 4 * q], t[o + 3 + 4 * q], t[o + 4 + 4 * q], t[o + 5 + 4 * q]]);
                let bone = u16::from_le_bytes([t[o], t[o + 1]]);
                println!("   ref {k:3}: bone {bone:3} a ({:.3}, {:.3}, {:.3}) b ({:.3}, {:.3}, {:.3})", f(0), f(1), f(2), f(3), f(4), f(5));
            }
        }
        if a.iter().any(|x| x == "--hex") {
            for (i, row) in t.chunks(26).enumerate().take(40) {
                println!("  {:4}: {:02x?}", i * 26, row);
            }
        }
        break;
    }
}
