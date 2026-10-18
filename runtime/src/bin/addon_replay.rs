//! Run a pinned addon against explicit simulated state; never opens FFXI.
fn main() -> std::process::ExitCode {
    let run = || -> Result<(), String> {
        use std::{io::Write, path::Path};
        let args: Vec<_> = std::env::args().skip(1).collect();
        if args.len() != 2 {
            return Err("Usage: addon-replay FIXTURE.json NEW-REPORT.json".into());
        }
        if Path::new(&args[1]).exists() {
            return Err("Choose a new replay report".into());
        }
        let fixture = windower_runtime::replay_host::read_fixture(Path::new(&args[0]))?;
        let result = windower_runtime::replay_host::run(fixture)?;
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args[1])
            .map_err(|e| e.to_string())?;
        out.write_all(&serde_json::to_vec_pretty(&result).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        out.sync_all().map_err(|e| e.to_string())?;
        Ok(())
    };
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            std::process::ExitCode::FAILURE
        }
    }
}
