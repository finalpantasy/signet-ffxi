//! A made-up install and a matching fake patch server, for tests and
//! screenshots. Built only with the `fake-server` feature; never part of a
//! release launcher. Nothing here touches the network beyond 127.0.0.1.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::catalog::Signature;
use crate::codec::flag;
use crate::error::{Result, io_at};

pub const OLD: &str = "30261001_0";
pub const NEW: &str = "30261102_1";

/// What the fake server holds and where the fake install is.
pub struct Fake {
    pub install: PathBuf,
    /// The version the server reports.
    pub version: String,
    pub catalog: String,
    pub files: HashMap<String, Vec<u8>>,
    /// Bytes the update downloads.
    pub download_bytes: u64,
}

fn bytes(seed: u32, n: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2_654_435_761) | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 11) as u8
        })
        .collect()
}

/// `big_files` data files of `big_kb` KB each change between OLD and NEW.
/// With `current` the server is at OLD and the install already matches it.
pub fn build(root: &Path, current: bool, big_files: usize, big_kb: usize) -> Result<Fake> {
    let install = root.join("SquareEnix").join("FINAL FANTASY XI");
    let mut text = String::new();
    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    let mut download = 0u64;
    let top = if current { OLD } else { NEW };

    // (path, old data installed, new data on the server)
    let mut set: Vec<(String, Option<Vec<u8>>, Option<Vec<u8>>)> = Vec::new();
    set.push(("VTABLE.DAT".into(), Some(bytes(1, 900)), None));
    set.push(("FTABLE.DAT".into(), Some(bytes(2, 700)), None));
    set.push(("FFXiMain.dll".into(), Some(bytes(3, 40_000)), Some(bytes(30, 41_000))));
    for i in 0..big_files {
        let n = big_kb * 1024 + i * 4096;
        set.push((format!("ROM/{}/{}.DAT", i + 1, i), Some(bytes(10 + i as u32, n)), Some(bytes(110 + i as u32, n + 8192))));
    }
    set.push(("ROM/20/0.DAT".into(), None, Some(bytes(77, 120_000))));
    for i in 0..30 {
        set.push((format!("ROM/9/{i}.DAT"), Some(bytes(200 + i, 64)), None));
    }

    for (path, old, new) in &set {
        let new = if current { &None } else { new };
        text.push_str(&format!("file {path} {{\n"));
        let mut line = |stamp: &str, data: &[u8], text: &mut String| {
            let s = Signature::of_bytes(data);
            let image_path = format!("{stamp}/Direct/{path}.slc");
            let mut image = vec![flag::STORED];
            image.extend_from_slice(data);
            text.push_str(&format!("{stamp} {} {} {} {image_path} {}\n", s.size, s.byte_sum, s.md5_head, image.len()));
            files.insert(image_path, image.clone());
            image.len() as u64
        };
        if let Some(d) = old {
            line(OLD, d, &mut text);
        }
        if let Some(d) = new {
            download += line(NEW, d, &mut text);
        }
        text.push_str("}\n\n");
    }
    text.push_str("end\n");

    let put = |rel: &str, data: &[u8]| -> Result<()> {
        let p = install.join(rel);
        io_at(fs::create_dir_all(p.parent().unwrap()), &p)?;
        io_at(fs::write(&p, data), &p)
    };
    for (path, old, _) in &set {
        if let Some(d) = old {
            put(path, d)?;
        }
    }
    put("patch.cfg", format!("file ROM/9/0.DAT {{\n{OLD} 64 1 1 {OLD}/Direct/ROM/9/0.DAT.slc 65\n}}\nend\n").as_bytes())?;
    Ok(Fake { install, version: top.to_string(), catalog: text, files, download_bytes: download })
}

/// Start the loopback server for `fake` on 127.0.0.1:`port` (0 picks a port).
/// Returns the address clients should use.
pub fn serve(port: u16, delay: std::time::Duration, fake: &Fake) -> crate::source::LiveServer {
    crate::source::loopback::serve_at(&format!("127.0.0.1:{port}"), delay, &fake.version, fake.catalog.as_bytes(), fake.files.clone())
}
