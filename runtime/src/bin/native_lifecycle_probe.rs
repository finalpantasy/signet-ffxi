#[cfg(all(windows, target_pointer_width = "32"))]
fn main() -> std::process::ExitCode {
    let run = || -> Result<(), String> {
        use std::io::Write;
        let args: Vec<_> = std::env::args().skip(1).collect();
        if args.len() != 3 {
            return Err("Usage: native-lifecycle-probe FIXTURE.dll SHA256 NEW-REPORT.json".into());
        }
        if std::path::Path::new(&args[2]).exists() {
            return Err("Choose a new native report".into());
        }
        let result = windower_runtime::plugin_lifecycle::verify_fixture(
            std::path::Path::new(&args[0]),
            &args[1],
        )?;
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args[2])
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
#[cfg(not(all(windows, target_pointer_width = "32")))]
fn main() {
    eprintln!("Requires Windows x86");
    std::process::exit(1);
}
