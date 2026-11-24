//! Exports every texture chunk of one DAT to PNG files.
//! Usage: signet-dat-textures <file-id | path-to-.DAT> <out-dir> [install-root]

use std::path::PathBuf;

use signet_dat::{Install, chunks, find_install_root, png, texture};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (target, out) = (&args[0], PathBuf::from(&args[1]));
    let data = if let Ok(id) = target.parse::<u32>() {
        let root = args.get(2).map(PathBuf::from).or_else(find_install_root).expect("no install");
        Install::open(root).expect("open install").read(id).expect("read DAT")
    } else {
        std::fs::read(target).expect("read DAT")
    };
    std::fs::create_dir_all(&out).expect("create out dir");
    let (mut ok, mut failed) = (0, 0);
    let textures = chunks(&data).filter_map(Result::ok).filter(|c| c.kind == texture::KIND);
    for (i, c) in textures.enumerate() {
        match texture::decode(c.data) {
            Ok((h, img)) => {
                let name = format!(
                    "{i:03}-{}-{}-{:?}-{}x{}.png",
                    h.group, h.name, h.format, h.width, h.height
                )
                .replace([' ', '/', '\\'], "_");
                let png = png::encode_rgba(img.width, img.height, &img.rgba);
                std::fs::write(out.join(name), png).expect("write png");
                ok += 1;
            }
            Err(e) => {
                eprintln!("chunk {} at {}: {e}", c.name_str(), c.offset);
                failed += 1;
            }
        }
    }
    println!("textures written: {ok}  failed: {failed}  -> {}", out.display());
}
