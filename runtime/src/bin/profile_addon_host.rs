//! Isolated JSON-lines lifecycle workbench; captures input instead of sending it to FFXI.
use serde::Deserialize;
use std::{
    io::{self, BufRead, Read, Write},
    path::Path,
    time::Duration,
};
use windower_rs::{
    addon_profile::AddonProfile,
    commands::{Backend, Input},
};
use windower_runtime::profile_addons::{Control, ProfileHost};

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Load { name: String },
    Reload { name: String },
    Unload { name: String },
    Tick { elapsed_ms: u64 },
    Status {},
    Shutdown {},
}
#[derive(Default)]
struct Capture(Vec<serde_json::Value>);
impl Backend for Capture {
    fn input(&mut self, input: &Input) -> Result<(), String> {
        self.0
            .push(serde_json::json!({"origin":input.origin,"mode":input.mode,"bytes":input.bytes}));
        Ok(())
    }
}
fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 1 {
        return Err("Usage: profile-addon-host PROFILE.json (JSON-lines controls on stdin)".into());
    }
    let mut host = ProfileHost::new(AddonProfile::read(Path::new(&args[0]))?)?;
    let mut capture = Capture::default();
    let mut output = io::stdout().lock();
    writeln!(output,"{}",serde_json::json!({"stage":"ready","autoload_errors":host.autoload(),"status":host.status(),"game_commands_sent":0})).map_err(|e|e.to_string())?;
    output.flush().map_err(|e| e.to_string())?;
    let mut input = io::stdin().lock();
    loop {
        let mut line = Vec::new();
        let count = input
            .by_ref()
            .take(65537)
            .read_until(b'\n', &mut line)
            .map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        if count > 65536 {
            return Err("Control exceeds 64 KiB; stream closed".into());
        }
        let request: Result<Request, _> = serde_json::from_slice(&line);
        let mut shutdown = false;
        let result = request
            .map_err(|e| e.to_string())
            .and_then(|request| match request {
                Request::Load { name } => host.apply(Control::Load { name }),
                Request::Reload { name } => host.apply(Control::Reload { name }),
                Request::Unload { name } => host.apply(Control::Unload { name }),
                Request::Tick { elapsed_ms } => {
                    host.tick(Duration::from_millis(elapsed_ms), &mut capture)
                }
                Request::Status {} => Ok(()),
                Request::Shutdown {} => {
                    shutdown = true;
                    host.shutdown();
                    Ok(())
                }
            });
        writeln!(output,"{}",serde_json::json!({"ok":result.is_ok(),"error":result.err(),"status":host.status(),"inputs":std::mem::take(&mut capture.0),"provider_counts":host.provider_counts()?,"game_commands_sent":0})).map_err(|e|e.to_string())?;
        output.flush().map_err(|e| e.to_string())?;
        if shutdown {
            break;
        }
    }
    host.shutdown();
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zero_field_controls_refuse_extra_fields() {
        for action in ["status", "shutdown"] {
            assert!(
                serde_json::from_value::<Request>(serde_json::json!({"action":action})).is_ok()
            );
            assert!(
                serde_json::from_value::<Request>(
                    serde_json::json!({"action":action,"unexpected":true})
                )
                .is_err()
            );
        }
    }
}
