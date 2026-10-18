use sha2::{Digest, Sha256};
use std::{env, fs, io::Write, path::Path, process::ExitCode};
use windower_rs::{
    profile::Settings,
    protocol::{LaunchContext, encode_settings},
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("packet-replay") if args.len() == 3 => {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Fixture {
                rules: Vec<windower_rs::packet_replay::Rule>,
                packets: Vec<windower_rs::packet_replay::Packet>,
            }
            if Path::new(&args[2]).exists() {
                return Err("Choose a new replay report".into());
            }
            let fixture: Fixture = read_bounded_json(Path::new(&args[1]), 8 * 1024 * 1024)?;
            if fixture.packets.len() > 1024 {
                return Err("Packet replay count limit".into());
            }
            let mut pipeline = windower_rs::packet_replay::Pipeline::default();
            for rule in fixture.rules {
                pipeline.register(rule)?;
            }
            let results = fixture
                .packets
                .into_iter()
                .map(|p| pipeline.process(p))
                .collect::<Result<Vec<_>, _>>()?;
            write_new(
                &args[2],
                &serde_json::to_vec_pretty(
                    &serde_json::json!({"results":results,"network_packets_sent":0}),
                )
                .map_err(|e| e.to_string())?,
            )?;
        }
        Some("package-resolve") if args.len() == 4 => {
            if Path::new(&args[3]).exists() {
                return Err("Choose a new package plan".into());
            }
            let catalog: Vec<windower_rs::packages::Manifest> =
                read_bounded_json(Path::new(&args[1]), 8 * 1024 * 1024)?;
            let roots: Vec<(String, String)> = read_bounded_json(Path::new(&args[2]), 65536)?;
            let plan = windower_rs::packages::resolve(&catalog, &roots)?;
            write_new(
                &args[3],
                &serde_json::to_vec_pretty(&plan).map_err(|e| e.to_string())?,
            )?;
        }
        Some("package-stage") if args.len() == 5 => {
            if Path::new(&args[4]).exists() {
                return Err("Choose a new package receipt".into());
            }
            let manifest = windower_rs::packages::Manifest::read(Path::new(&args[2]))?;
            let receipt = windower_rs::packages::Store::open(Path::new(&args[1]))?
                .stage(&manifest, Path::new(&args[3]))?;
            write_new(
                &args[4],
                &serde_json::to_vec_pretty(&receipt).map_err(|e| e.to_string())?,
            )?;
        }
        Some("package-activate") if args.len() == 4 => {
            if Path::new(&args[3]).exists() {
                return Err("Choose a new activation receipt".into());
            }
            let slots: Vec<String> = read_bounded_json(Path::new(&args[2]), 131072)?;
            let generation =
                windower_rs::packages::Store::open(Path::new(&args[1]))?.activate(&slots)?;
            write_new(
                &args[3],
                &serde_json::to_vec_pretty(&generation).map_err(|e| e.to_string())?,
            )?;
        }
        Some("package-rollback") if args.len() == 4 => {
            if Path::new(&args[3]).exists() {
                return Err("Choose a new rollback receipt".into());
            }
            let number = args[2]
                .parse::<u64>()
                .map_err(|_| "Invalid generation number")?;
            let generation =
                windower_rs::packages::Store::open(Path::new(&args[1]))?.rollback(number)?;
            write_new(
                &args[3],
                &serde_json::to_vec_pretty(&generation).map_err(|e| e.to_string())?,
            )?;
        }
        Some("addon-control") if args.len() == 5 => {
            #[cfg(windows)]
            {
                if Path::new(&args[4]).exists() {
                    return Err("Choose a new control reply report".into());
                }
                let sequence = args[2]
                    .parse::<u64>()
                    .map_err(|_| "Invalid control sequence")?;
                let reply = windower_rs::addon_control_client::send(
                    Path::new(&args[1]),
                    sequence,
                    Path::new(&args[3]),
                )?;
                write_new(
                    &args[4],
                    &serde_json::to_vec_pretty(&reply).map_err(|e| e.to_string())?,
                )?;
                println!("Addon control reply saved; accepted={}", reply["ok"]);
            }
            #[cfg(not(windows))]
            return Err("Live addon controls require Windows".into());
        }
        Some("addon-profile-check") if args.len() == 2 => {
            let profile = windower_rs::addon_profile::AddonProfile::read(Path::new(&args[1]))?;
            let mut sources = std::collections::BTreeMap::new();
            for spec in &profile.addons {
                let source = profile.resolve(&spec.name)?;
                sources.insert(source.name, source.sha256);
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&sources).map_err(|e| e.to_string())?
            );
        }
        Some("capabilities") if args.len() == 1 => {
            println!(
                "{}",
                serde_json::to_string_pretty(windower_rs::platform::CAPABILITIES)
                    .map_err(|error| error.to_string())?
            );
        }
        Some("clients") if args.len() == 1 => {
            #[cfg(windows)]
            println!(
                "{}",
                serde_json::to_string_pretty(&windower_rs::discovery::discover()?)
                    .map_err(|error| error.to_string())?
            );
            #[cfg(not(windows))]
            return Err("Client discovery requires Windows Registry32".into());
        }
        Some("analyze-code") if args.len() == 3 => {
            if Path::new(&args[2]).exists() {
                return Err("Refusing to replace an existing analysis report".into());
            }
            let capture = windower_rs::client_code::LoadedCodeCapture::read(Path::new(&args[1]))?;
            write_new(
                &args[2],
                &serde_json::to_vec_pretty(&capture.report).map_err(|error| error.to_string())?,
            )?;
            println!(
                "Analyzed {} executable sections; {} patched candidates",
                capture.code.len(),
                capture.report.patched_candidates.len()
            );
        }
        Some("capture-client") if args.len() == 3 => {
            #[cfg(all(windows, target_pointer_width = "32"))]
            {
                let directory = Path::new(&args[2]);
                if directory.exists() {
                    return Err("Refusing to replace an existing code capture".into());
                }
                let process_id = args[1].parse::<u32>().map_err(|_| "Invalid process ID")?;
                if process_id == 0 {
                    return Err("A positive process ID is required".into());
                }
                let capture = windower_rs::client_code::capture_loaded_client(process_id)?;
                capture.write_new(directory)?;
                println!(
                    "Saved {} executable sections to {}",
                    capture.code.len(),
                    directory.display()
                );
            }
            #[cfg(not(all(windows, target_pointer_width = "32")))]
            return Err("Build capture-client for i686-pc-windows-msvc".into());
        }
        Some("scan-client") if args.len() == 3 => {
            #[cfg(all(windows, target_pointer_width = "32"))]
            {
                if Path::new(&args[2]).exists() {
                    return Err("Refusing to replace an existing code report".into());
                }
                let process_id = args[1].parse::<u32>().map_err(|_| "Invalid process ID")?;
                if process_id == 0 {
                    return Err("A positive process ID is required".into());
                }
                let report = windower_rs::client_code::scan_loaded_client(process_id)?;
                let bytes =
                    serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
                write_new(&args[2], &bytes)?;
                println!(
                    "Read {} executable sections; report: {}",
                    report.sections.len(),
                    args[2]
                );
            }
            #[cfg(not(all(windows, target_pointer_width = "32")))]
            return Err("Build scan-client for i686-pc-windows-msvc".into());
        }
        Some("launch-local") if args.len() == 6 => {
            #[cfg(all(windows, target_pointer_width = "32"))]
            {
                let packet = fs::read(&args[2]).map_err(|e| e.to_string())?;
                let settings = windower_rs::protocol::decode_settings(&packet)?;
                let options: windower_rs::runtime_options::RuntimeOptions =
                    serde_json::from_slice(&fs::read(&args[3]).map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())?;
                options.validate()?;
                if !options.virtual_display
                    || settings.fullscreen
                    || !settings.init_addons.is_empty()
                    || !settings.init_plugins.is_empty()
                    || !options.addons.is_empty()
                    || options.luajit_provider.is_some()
                {
                    return Err("Local trial requires virtual windowed display and empty addon/plugin selections".into());
                }
                let diagnostic = options
                    .client_diagnostic
                    .as_ref()
                    .ok_or("Local trial requires a first-frame diagnostic")?;
                if Path::new(&args[5]).exists()
                    || diagnostic.exists()
                    || options
                        .game_thread_probe
                        .as_ref()
                        .is_some_and(|p| p.ready_report.exists() || p.result_report.exists())
                {
                    return Err("Local trial report already exists".into());
                }
                let local = windower_rs::local_client::read(Path::new(&args[4]))?;
                local.verify()?;
                let discovered = windower_rs::discovery::discover()?;
                let region = match settings.region {
                    0 => windower_rs::profile::Region::JP,
                    1 => windower_rs::profile::Region::US,
                    2 => windower_rs::profile::Region::EU,
                    _ => return Err("Invalid local client region".into()),
                };
                let installed = windower_rs::discovery::select_region(&discovered.clients, region)?;
                if Path::new(&settings.ffxi_path)
                    .canonicalize()
                    .map_err(|e| e.to_string())?
                    != installed
                        .ffxi_path
                        .as_ref()
                        .ok_or("Missing installed FFXI path")?
                        .canonicalize()
                        .map_err(|e| e.to_string())?
                {
                    return Err("Local trial game path differs from installed client".into());
                }
                windower_rs::display::DisplayValues::from_settings(&settings)?;
                let child = windower_rs::launch::launch_interactive_client(
                    &local.loader.path,
                    Path::new(&args[1]),
                    &packet,
                    &local.arguments(),
                    &options,
                )?;
                write_new(&args[5],&serde_json::to_vec_pretty(&serde_json::json!({"stage":"local_loopback_client_launch","child_pid":child.process_id,"loopback_only":true,"virtual_display":true,"game_integration_complete":false,"runtime":args[1]})).map_err(|e|e.to_string())?)?;
                println!(
                    "Started separate Rust local client: PID {}",
                    child.process_id
                );
            }
            #[cfg(not(all(windows, target_pointer_width = "32")))]
            return Err("Build launch-local for i686-pc-windows-msvc".into());
        }
        Some("launch-client") if args.len() == 5 => {
            #[cfg(all(windows, target_pointer_width = "32"))]
            {
                if Path::new(&args[4]).exists() {
                    return Err("Refusing to replace an existing client launch report".into());
                }
                let packet = fs::read(&args[2]).map_err(|error| error.to_string())?;
                let settings = windower_rs::protocol::decode_settings(&packet)?;
                if !settings.init_plugins.is_empty() || !settings.init_addons.is_empty() {
                    return Err(
                        "First client diagnostic requires empty addon/plugin startup lists".into(),
                    );
                }
                let options: windower_rs::runtime_options::RuntimeOptions =
                    serde_json::from_slice(&fs::read(&args[3]).map_err(|error| error.to_string())?)
                        .map_err(|error| error.to_string())?;
                options.validate()?;
                let diagnostic = options
                    .client_diagnostic
                    .as_ref()
                    .ok_or("First client launch requires an explicit diagnostic report")?;
                if diagnostic.exists() {
                    return Err("Client diagnostic report already exists".into());
                }
                if let Some(probe) = &options.game_thread_probe
                    && (probe.ready_report.exists() || probe.result_report.exists())
                {
                    return Err("Game thread probe report already exists".into());
                }
                let discovered = windower_rs::discovery::discover()?;
                let region = match settings.region {
                    0 => windower_rs::profile::Region::JP,
                    1 => windower_rs::profile::Region::US,
                    2 => windower_rs::profile::Region::EU,
                    _ => return Err("Invalid client region".into()),
                };
                let installation =
                    windower_rs::discovery::select_region(&discovered.clients, region)?;
                let directory = installation
                    .play_online_path
                    .as_ref()
                    .ok_or("Missing installed PlayOnline path")?
                    .canonicalize()
                    .map_err(|error| error.to_string())?;
                if Path::new(&settings.play_online_path)
                    .canonicalize()
                    .map_err(|error| error.to_string())?
                    != directory
                {
                    return Err("Packet PlayOnline path differs from installed client".into());
                }
                let child = windower_rs::launch::launch_interactive_client(
                    &directory.join("pol.exe"),
                    Path::new(&args[1]),
                    &packet,
                    &["/game".into(), "eAZcFcB".into()],
                    &options,
                )?;
                let report = serde_json::to_vec_pretty(&serde_json::json!({"stage":"new_client_diagnostic_launch","child_pid":child.process_id,
                    "runtime":args[1],"client_diagnostic":diagnostic,"game_integration_complete":false})).map_err(|error| error.to_string())?;
                write_new(&args[4], &report)?;
                println!(
                    "Started separate PlayOnline client: PID {}",
                    child.process_id
                );
            }
            #[cfg(not(all(windows, target_pointer_width = "32")))]
            return Err("Build launch-client for i686-pc-windows-msvc".into());
        }
        Some("launch-probe" | "launch-probe-interactive") if args.len() == 5 || args.len() == 6 => {
            #[cfg(all(windows, target_pointer_width = "32"))]
            {
                let requested = Path::new(&args[4]);
                let parent = requested
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(Path::new("."));
                let report = parent
                    .canonicalize()
                    .map_err(|e| e.to_string())?
                    .join(requested.file_name().ok_or("Missing report filename")?);
                if report.exists() {
                    return Err("Refusing to replace an existing test report".into());
                }
                let runtime = Path::new(&args[1])
                    .canonicalize()
                    .map_err(|e| e.to_string())?;
                let packet = fs::read(&args[2]).map_err(|e| e.to_string())?;
                let options = if args.len() == 6 {
                    let options: windower_rs::runtime_options::RuntimeOptions =
                        serde_json::from_slice(
                            &fs::read(&args[5]).map_err(|error| error.to_string())?,
                        )
                        .map_err(|error| error.to_string())?;
                    options
                } else {
                    windower_rs::runtime_options::RuntimeOptions::default()
                };
                let arguments = [
                    "--injected".into(),
                    runtime.as_os_str().into(),
                    report.as_os_str().into(),
                ];
                let child = if args[0] == "launch-probe-interactive" {
                    let client = windower_rs::launch::launch_interactive_client(
                        Path::new(&args[3]),
                        &runtime,
                        &packet,
                        &arguments,
                        &options,
                    )?;
                    let Some(exit_code) = client.wait_for_exit(30000)? else {
                        client.terminate()?;
                        return Err("Interactive-startup fixture timed out".into());
                    };
                    if exit_code != 0 {
                        return Err(format!(
                            "Interactive-startup fixture failed: {exit_code:#010x}"
                        ));
                    }
                    windower_rs::launch::ChildResult {
                        process_id: client.process_id,
                        exit_code,
                    }
                } else {
                    windower_rs::launch::run_isolated_child_with_options(
                        Path::new(&args[3]),
                        &runtime,
                        &packet,
                        &arguments,
                        &options,
                    )?
                };
                let graphics: serde_json::Value =
                    serde_json::from_slice(&fs::read(&report).map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())?;
                println!(
                    "{}",
                    serde_json::json!({"stage":"suspended_child_runtime_launch","child_pid":child.process_id,"child_exit_code":child.exit_code,"graphics":graphics,"runtime_options":options,"interactive_startup":args[0] == "launch-probe-interactive","game_integration_complete":false})
                );
            }
            #[cfg(not(all(windows, target_pointer_width = "32")))]
            return Err("Build launch-probe for i686-pc-windows-msvc".into());
        }
        Some("profiles") if args.len() == 2 => {
            let xml = fs::read_to_string(&args[1]).map_err(|e| e.to_string())?;
            let settings = Settings::parse(&xml)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?
            );
        }
        Some("inspect") if args.len() == 2 => {
            let root = Path::new(&args[1]);
            let xml = fs::read_to_string(root.join("settings.xml")).map_err(|e| e.to_string())?;
            let settings = Settings::parse(&xml)?;
            let files = ["Windower.exe", "Hook.dll", "plugins/LuaCore.dll"]
                .into_iter().map(|name| {
                    match fs::read(root.join(name)) {
                        Ok(bytes) => serde_json::json!({"file":name,"bytes":bytes.len(),"sha256":format!("{:x}", Sha256::digest(bytes))}),
                        Err(error) => serde_json::json!({"file":name,"error":error.to_string()}),
                    }
                }).collect::<Vec<_>>();
            println!("{}", serde_json::to_string_pretty(&serde_json::json!({
                "status":"Partial port: profile/protocol workbench and runtime bootstrap; no game integration yet",
                "profiles":settings.profiles.len(),"enabled_addons":settings.addons,
                "enabled_plugins":settings.plugins,"files":files,
            })).map_err(|e| e.to_string())?);
        }
        Some("packet") if args.len() == 4 => {
            // Test/export command. Does not open shared memory or start a process.
            let input: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&args[1]).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let xml = input["xml"].as_str().ok_or("Fixture missing xml")?;
            let name = input["profile"].as_str().unwrap_or("");
            let c = &input["context"];
            let context = LaunchContext {
                branch: string(c, "branch")?,
                launcher_version: string(c, "launcher_version")?,
                hook_version: string(c, "hook_version")?,
                play_online_path: string(c, "play_online_path")?,
                ffxi_path: string(c, "ffxi_path")?,
                windower_path: string(c, "windower_path")?,
                region: match c["region"].as_str() {
                    Some("JP") => windower_rs::profile::Region::JP,
                    Some("US") => windower_rs::profile::Region::US,
                    Some("EU") => windower_rs::profile::Region::EU,
                    _ => return Err("Invalid context region".into()),
                },
                display_width: c["display_width"]
                    .as_i64()
                    .and_then(|v| i32::try_from(v).ok())
                    .ok_or("Invalid width")?,
                display_height: c["display_height"]
                    .as_i64()
                    .and_then(|v| i32::try_from(v).ok())
                    .ok_or("Invalid height")?,
                plugins: names(c, "plugins")?,
                addons: names(c, "addons")?,
            };
            let settings = Settings::parse(xml)?;
            let profile = settings.select(name);
            let bytes = encode_settings(&profile, &context)?;
            for path in [&args[2], &args[3]] {
                if Path::new(path).exists() {
                    return Err(format!("Refusing to overwrite {path}"));
                }
            }
            let profile_json = serde_json::to_string_pretty(&profile).map_err(|e| e.to_string())?;
            write_new(&args[2], &bytes)?;
            write_new(&args[3], profile_json.as_bytes())?;
            println!("{} bytes, sha256 {:x}", bytes.len(), Sha256::digest(&bytes));
        }
        Some("--help") | None => println!(
            "windower-rs (partial port; FFXI integration incomplete)\n  clients\n  inspect <Windower directory>\n  profiles <settings.xml>\n  packet <fixture.json> <new packet.bin> <new profile.json>\n  launch-probe <Rust DLL> <settings packet> <graphics-probe.exe> <new report.json>\n  scan-client <i686 PID with FFXiMain.dll loaded> <new code report.json>\n  capture-client <i686 PID> <new executable-code directory>\n  analyze-code <executable-code directory> <new report.json>"
        ),
        _ => return Err("Unknown command; use --help".into()),
    }
    Ok(())
}
fn read_bounded_json<T: serde::de::DeserializeOwned>(path: &Path, limit: u64) -> Result<T, String> {
    use std::io::Read;
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("JSON document byte limit exceeded".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}
fn write_new(path: &str, bytes: &[u8]) -> Result<(), String> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut file| file.write_all(bytes))
        .map_err(|e| format!("{path}: {e}"))
}
fn string(c: &serde_json::Value, name: &str) -> Result<String, String> {
    c[name]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("Invalid {name}"))
}
fn names(c: &serde_json::Value, name: &str) -> Result<Vec<String>, String> {
    c[name]
        .as_array()
        .ok_or_else(|| format!("Invalid {name}"))?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("Invalid {name} entry"))
        })
        .collect()
}
