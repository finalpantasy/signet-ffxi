//! signet-parity-compare <retail.csv> <signet.csv>
//!
//! Prints a table of retail value, Signet value and difference for every measured behavior, with
//! the retail value to use for each Signet constant that is still UNVERIFIED.
//! `--make-synthetic <dir>` writes a made-up retail-like and a Signet-like recording there, to
//! try the tool without a real recording.

use signet_parity::{compare, rows, synth};
use std::process::ExitCode;

fn load(p: &str) -> Result<Vec<rows::Row>, String> {
    let text = std::fs::read_to_string(p).map_err(|e| format!("read {p}: {e}"))?;
    rows::read(&text).map_err(|e| format!("{p}: {e}"))
}

fn write(p: &std::path::Path, v: &[rows::Row]) -> Result<(), String> {
    let mut w = rows::Writer::create(p)?;
    for r in v {
        w.push(r)?;
    }
    w.flush()
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--make-synthetic") {
        let dir = std::path::PathBuf::from(args.get(1).ok_or("--make-synthetic needs a directory")?);
        write(&dir.join("synthetic-retail.csv"), &synth::generate(&synth::Params::retail_like()))?;
        write(&dir.join("synthetic-signet.csv"), &synth::generate(&synth::Params::signet_like()))?;
        println!("wrote {} and {}", dir.join("synthetic-retail.csv").display(), dir.join("synthetic-signet.csv").display());
        return Ok(());
    }
    if args.len() != 2 || args[0].starts_with("--") {
        return Err("usage: signet-parity-compare <retail.csv> <signet.csv>\n       signet-parity-compare --make-synthetic <dir>".into());
    }
    let (retail, signet) = (load(&args[0])?, load(&args[1])?);
    print!("{}", compare::report(&compare::metrics(&retail), &compare::metrics(&signet)));
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("signet-parity-compare: {e}");
            ExitCode::from(2)
        }
    }
}
