//! Embeds the built UI (`ui/launcher/dist`) into the exe as a static table.
//! Build the UI first (`npm run build` in ui/launcher); without it the exe shows a notice page.

use std::path::{Path, PathBuf};

fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.path());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            walk(&p, base, out);
        } else if let Ok(rel) = p.strip_prefix(base) {
            let rel = rel.to_string_lossy().replace('\\', "/");
            out.push((rel, p.canonicalize().unwrap_or(p.clone())));
        }
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let dist = manifest.join("..").join("..").join("ui").join("launcher").join("dist");
    println!("cargo:rerun-if-changed={}", dist.display());
    let mut files = Vec::new();
    walk(&dist, &dist, &mut files);
    for (_, p) in &files {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    let mut src = String::from("pub static ASSETS: &[(&str, &[u8])] = &[\n");
    for (rel, p) in &files {
        let abs = p.to_string_lossy().trim_start_matches(r"\\?\").to_string();
        src.push_str(&format!("    ({rel:?}, include_bytes!({abs:?})),\n"));
    }
    src.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("assets.rs");
    std::fs::write(out, src).unwrap();
}
