//! A fake patch server and fake install on 127.0.0.1, for testing the launcher
//! without Square Enix's servers. Build with `--features fake-server`.
//!
//! signet-patch-fake <root> --port N [--current] [--files N] [--kb N] [--delay-ms N]
//!
//! Writes the fake install under <root>\SquareEnix\FINAL FANTASY XI, prints
//! `install=<path>` and `ready` on stdout, then serves until killed.

use std::path::PathBuf;
use std::time::Duration;

use signet_patch::fake;

fn main() {
    let mut a = std::env::args().skip(1);
    let root: PathBuf = a.next().expect("usage: signet-patch-fake <root> --port N").into();
    let (mut port, mut current, mut files, mut kb, mut delay) = (53999u16, false, 6usize, 1500usize, 0u64);
    while let Some(o) = a.next() {
        let mut num = |n: &str| a.next().and_then(|v| v.parse::<u64>().ok()).unwrap_or_else(|| panic!("{n} needs a number"));
        match o.as_str() {
            "--port" => port = num("--port") as u16,
            "--current" => current = true,
            "--files" => files = num("--files") as usize,
            "--kb" => kb = num("--kb") as usize,
            "--delay-ms" => delay = num("--delay-ms"),
            _ => panic!("unknown option {o}"),
        }
    }
    let built = fake::build(&root, current, files, kb).expect("build the fake install");
    signet_patch::fake::serve(port, Duration::from_millis(delay), &built);
    println!("install={}", built.install.display());
    println!("version={}", built.version);
    println!("download_bytes={}", built.download_bytes);
    println!("ready");
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
