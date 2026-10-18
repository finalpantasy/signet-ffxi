//! Native Rust runtime bootstrap. Game hooks and plugin hosting are still pending.
//! This DLL never loads the original Hook.dll or LuaCore.dll.

#[cfg(all(windows, target_pointer_width = "32"))]
mod controller_discovery;
#[cfg(windows)]
mod native;

#[cfg(all(windows, target_pointer_width = "32", feature = "game-thread-probe"))]
mod addon_echo_trial;
#[cfg(all(windows, target_pointer_width = "32", feature = "client-diagnostics"))]
mod client_diagnostics;
#[cfg(all(windows, target_pointer_width = "32", feature = "client-windowing"))]
mod display;
#[cfg(all(windows, target_pointer_width = "32", feature = "game-thread-probe"))]
mod fixed_hooks;
pub mod game_addons;
#[cfg(all(windows, target_pointer_width = "32"))]
pub mod game_bridge;
#[cfg(all(windows, target_pointer_width = "32", feature = "game-thread-probe"))]
mod game_thread_probe;
#[cfg(all(windows, target_pointer_width = "32", feature = "game-thread-probe"))]
mod lab_login;
#[cfg(all(windows, target_pointer_width = "32", feature = "game-thread-probe"))]
mod lab_reload;
#[cfg(test)]
mod lab_server_tests;
#[cfg(all(windows, target_pointer_width = "32", feature = "game-thread-probe"))]
mod lab_session;
pub mod lua;
mod persistent_vm;
pub mod profile_addons;
pub mod profile_control;
#[cfg(feature = "isolated-probe")]
pub mod replay_host;

#[cfg(all(
    windows,
    target_pointer_width = "32",
    feature = "game-thread-probe",
    feature = "isolated-probe"
))]
pub fn run_file_reload_probe(
    options: &windower_rs::runtime_options::RuntimeOptions,
) -> Result<serde_json::Value, String> {
    use windower_rs::commands::{Backend, Input};
    options.validate()?;
    let lab = options
        .game_thread_probe
        .as_ref()
        .and_then(|p| p.lab_test.as_ref())
        .ok_or("Missing lab selection")?;
    let files = lab
        .file_profile
        .as_ref()
        .ok_or("Missing prepared file profile")?;
    let mut session = lab_reload::Session::new_files(&lab.nonce, lab.scenario, files)?;
    #[derive(Default)]
    struct Capture(Vec<String>);
    impl Backend for Capture {
        fn input(&mut self, input: &Input) -> Result<(), String> {
            self.0
                .push(String::from_utf8(input.bytes.clone()).map_err(|e| e.to_string())?);
            Ok(())
        }
    }
    let mut capture = Capture::default();
    let mut times = Vec::new();
    let mut report = None;
    for ms in (0..=18000).step_by(100) {
        let before = capture.0.len();
        report = session.tick_at(std::time::Duration::from_millis(ms), &mut capture)?;
        if capture.0.len() != before {
            if capture.0.len() != before + 1 {
                return Err("File reload emitted a command burst".into());
            }
            times.push(ms);
        }
        if report.is_some() && ms != 18000 {
            return Err("File reload completed early".into());
        }
    }
    let steps = [
        "reload_old1",
        "reload_neighbor1",
        "reload_old2",
        "reload_new101",
        "reload_neighbor2",
        "reload_new102",
    ];
    let expected: Vec<_> = steps
        .into_iter()
        .map(|s| format!("/say !rustlab {} {s}", lab.nonce))
        .collect();
    if capture.0 != expected || times != [0, 3000, 6000, 9000, 12000, 15000] || report.is_none() {
        return Err("File reload transport/timing witnesses differ".into());
    }
    Ok(
        serde_json::json!({"checks_passed":true,"result":report,"inputs":capture.0,"times_ms":times,"game_commands_sent":0}),
    )
}

#[cfg(all(windows, target_pointer_width = "32"))]
pub mod plugin;

#[cfg(all(windows, target_pointer_width = "32"))]
pub mod ashita_plugin;
#[cfg(all(windows, target_pointer_width = "32", feature = "isolated-probe"))]
pub mod plugin_lifecycle;

#[cfg(all(windows, target_pointer_width = "32"))]
pub mod vm_client;

#[cfg(all(windows, target_pointer_width = "32"))]
mod graphics;

#[cfg(all(windows, feature = "isolated-probe"))]
pub mod probe_host;

#[cfg(all(windows, feature = "isolated-probe"))]
mod frame_host;

#[cfg(all(windows, target_pointer_width = "32", feature = "isolated-probe"))]
mod input;

#[cfg(windows)]
pub use native::{mapping_names, runtime_status};
