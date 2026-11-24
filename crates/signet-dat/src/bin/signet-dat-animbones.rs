//! Prints which bones an animation drives and where the high (attachment) bones end up.
//! Usage: signet-dat-animbones <skeleton-dat-id> <anim name>... [--from 80] [--root <install>]
use signet_dat::{Install, anim, chunks, find_install_root, skeleton};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let root = a.iter().position(|x| x == "--root").and_then(|i| a.get(i + 1)).map(Into::into).or_else(find_install_root).expect("no install root");
    let from: usize = a.iter().position(|x| x == "--from").and_then(|i| a.get(i + 1)).and_then(|s| s.parse().ok()).unwrap_or(80);
    let id: u32 = a[0].parse().unwrap();
    let extra_at = a.iter().position(|x| x == "--extra").unwrap_or(a.len());
    let names: Vec<&String> = a[1..extra_at].iter().filter(|x| !x.starts_with("--") && x.parse::<usize>().is_err()).collect();
    let install = Install::open(&root).expect("open install");
    let mut dats = vec![install.read(id).expect("read")];
    if let Some(i) = a.iter().position(|x| x == "--extra") {
        for x in a[i + 1..].iter().take_while(|x| !x.starts_with("--")) {
            dats.push(install.read(x.parse().unwrap()).expect("read extra"));
        }
    }
    let mut sk = None;
    let mut anims = Vec::new();
    for c in dats.iter().flat_map(|d| chunks(d).filter_map(Result::ok)) {
        if c.kind == skeleton::KIND && sk.is_none() {
            sk = skeleton::parse(c.data).ok();
        } else if c.kind == anim::KIND {
            if let Ok(an) = anim::parse(c.data) {
                anims.push((c.name_str().trim_end_matches('.').to_string(), an));
            }
        }
    }
    let sk = sk.unwrap();
    if a.iter().any(|x| x == "--scan") {
        for (n, an) in &anims {
            let d: Vec<usize> = an.tracks.iter().map(|t| t.bone).filter(|&b| b >= from).collect();
            if !d.is_empty() {
                println!("{n}: {} frames, drives {} bones from {} ({:?}..)", an.frames, d.len(), d[0], &d[..d.len().min(4)]);
            }
        }
        return;
    }
    let bind = sk.bind_local();
    for n in names {
        let Some((_, an)) = anims.iter().find(|(x, _)| x == n.as_str()) else { println!("no anim {n}"); continue };
        println!("anim {n}: {} frames speed {} tracks {}", an.frames, an.speed, an.tracks.len());
        let driven: Vec<usize> = an.tracks.iter().map(|t| t.bone).filter(|&b| b >= from).collect();
        println!("  driven bones >= {from}: {driven:?}");
        let local = an.local_poses(&bind, 0.0);
        let mut world = vec![skeleton::Pose::IDENTITY; local.len()];
        for i in 0..local.len() {
            world[i] = match sk.bones[i].parent {
                Some(p) => world[p].compose(&local[i]),
                None => local[i],
            };
        }
        for b in (from..sk.bones.len()).filter(|b| std::env::var("ONLY").map_or(true, |o| o.split(',').any(|x| x.parse::<usize>() == Ok(*b)))) {
            let w = world[b];
            println!("  bone {b} parent {:?}: world t ({:.3}, {:.3}, {:.3}) q ({:.3} {:.3} {:.3} {:.3})", sk.bones[b].parent, w.translation[0], w.translation[1], w.translation[2], w.rotation[0], w.rotation[1], w.rotation[2], w.rotation[3]);
        }
        // reference: pelvis(2), right hand-ish landmarks
        for b in [2usize, 58, 63, 73, 78] {
            if b < world.len() { println!("  (ref) bone {b}: t ({:.3}, {:.3}, {:.3})", world[b].translation[0], world[b].translation[1], world[b].translation[2]); }
        }
    }
}
