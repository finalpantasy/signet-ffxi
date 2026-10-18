//! Run prepared file fixtures on a synthetic clock; never starts or calls FFXI.
use std::{io::Write, path::Path};
fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        return Err("Usage: lab-file-reload-probe OPTIONS.json NEW-REPORT.json".into());
    }
    let bytes = std::fs::read(&args[0]).map_err(|e| e.to_string())?;
    if bytes.len() > 65536 {
        return Err("Options file too large".into());
    }
    let options: windower_rs::runtime_options::RuntimeOptions =
        serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let result = windower_runtime::run_file_reload_probe(&options)?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(Path::new(&args[1]))
        .map_err(|e| e.to_string())?;
    output
        .write_all(&serde_json::to_vec_pretty(&result).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    println!(
        "PASS: prepared file reload fixture; six ordered captures; typed unload; zero game commands."
    );
    Ok(())
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            std::process::ExitCode::FAILURE
        }
    }
}
