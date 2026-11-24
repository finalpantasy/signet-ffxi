//! Prints what is inside DATs by file id: chunk counts and the names of meshes, textures and animations.
//! Usage: signet-dat-idinfo <first-id> [<last-id>] [--root <install>]
use signet_dat::{Install, actor_mesh, anim, chunks, find_install_root, skeleton, texture};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let root = a.iter().position(|x| x == "--root").and_then(|i| a.get(i + 1)).map(Into::into).or_else(find_install_root).expect("no install root");
    let nums: Vec<u32> = a.iter().filter_map(|s| s.parse().ok()).collect();
    let (lo, hi) = (nums[0], *nums.get(1).unwrap_or(&nums[0]));
    let install = Install::open(&root).expect("open install");
    for id in lo..=hi {
        let Ok(dat) = install.read(id) else { continue };
        let (mut sk, mut meshes, mut anims, mut texs) = (0, Vec::new(), Vec::new(), Vec::new());
        let mut other = 0;
        for c in chunks(&dat).filter_map(Result::ok) {
            match c.kind {
                skeleton::KIND => sk += 1,
                actor_mesh::KIND => meshes.push(c.name_str()),
                anim::KIND => anims.push(c.name_str()),
                texture::KIND => texs.push(c.name_str()),
                0 => {}
                _ => other += 1,
            }
        }
        let cut = |v: &Vec<String>, n: usize| v.iter().take(n).cloned().collect::<Vec<_>>().join(",");
        println!(
            "{id} {} bytes sk {sk} mesh {} [{}] anim {} [{}] tex {} [{}] other {other}",
            dat.len(), meshes.len(), cut(&meshes, 4), anims.len(), cut(&anims, 4), texs.len(), cut(&texs, 3)
        );
    }
}
