//! Explicit one-shot local echo after a natural user command on the draw thread.
//! Optional embedded Lua fixtures stage three local echoes before hook install.
//! No arbitrary command transport, chat logging or packet hook.
use crate::fixed_hooks::{self, Patch};
use std::{
    cell::RefCell,
    ffi::{c_char, c_void},
    io::Write,
    path::Path,
    ptr,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU32, Ordering},
    },
};
use windower_rs::{client_code::ScanReport, runtime_options::GameThreadProbe};
use windows_sys::Win32::Foundation::{GetLastError, SetLastError};
use windows_sys::Win32::System::{
    Diagnostics::Debug::ReadProcessMemory,
    LibraryLoader::{GetModuleFileNameW, GetModuleHandleW},
    Threading::{
        CreateEventW, GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId, SetEvent,
        WaitForMultipleObjects, WaitForSingleObject,
    },
};
const MARKER: &[u8] = b"/echo RustBridge\0";
const RESPONSE: &[u8] = b"/echo Rust runtime command bridge verified\0";
const DISABLED: u32 = 0;
const WAITING: u32 = 1;
const EXECUTING: u32 = 2;
const COMPLETE: u32 = 3;
const WRONG_THREAD: u32 = 4;
const EXPIRED: u32 = 5;
const FAILED: u32 = 6;
const AUDITED_GAME_SHA256: &str =
    "042884888eea4fadac183e520f2ae9f8c4fa267860e685048e1880efd358e872";
type Input = unsafe extern "C" fn(*const c_char, i32) -> i32;
type Draw = unsafe extern "thiscall" fn(*const c_void);
struct Gate {
    phase: AtomicU32,
    marker_thread: AtomicU32,
    marker_mode: AtomicU32,
}
impl Gate {
    fn new() -> Self {
        Self {
            phase: AtomicU32::new(DISABLED),
            marker_thread: AtomicU32::new(0),
            marker_mode: AtomicU32::new(0),
        }
    }
    fn marker(&self, thread: u32, mode: i32) {
        if self.phase.load(Ordering::Acquire) != WAITING || !(0..=7).contains(&mode) {
            return;
        }
        // A marker is accepted once; subsequent commands cannot change its owner.
        if self
            .marker_thread
            .compare_exchange(0, u32::MAX, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.marker_mode.store(mode as u32, Ordering::Relaxed);
            self.marker_thread.store(thread, Ordering::Release);
        }
    }
    fn claim(&self, draw_thread: u32) -> Option<Result<i32, ()>> {
        let owner = self.marker_thread.load(Ordering::Acquire);
        if owner == 0 || owner == u32::MAX {
            return None;
        }
        let phase = if owner == draw_thread {
            EXECUTING
        } else {
            WRONG_THREAD
        };
        self.phase
            .compare_exchange(WAITING, phase, Ordering::AcqRel, Ordering::Acquire)
            .ok()?;
        Some(if phase == EXECUTING {
            Ok(self.marker_mode.load(Ordering::Acquire) as i32)
        } else {
            Err(())
        })
    }
}
struct Probe {
    gate: Gate,
    input: usize,
    draw: usize,
    event: usize,
    draw_calls: AtomicU32,
    natural_inputs: AtomicU32,
    draw_thread: AtomicU32,
    game_calls: AtomicU32,
    staged_inputs: Option<Vec<windower_rs::commands::Input>>,
    lab: Option<Lab>,
    lab_report: Mutex<Option<serde_json::Value>>,
}
struct Lab {
    config: windower_rs::runtime_options::LabTest,
    login: crate::lab_login::Login,
    request_event: usize,
    requested: std::sync::atomic::AtomicBool,
    world_probing: std::sync::atomic::AtomicBool,
    world_probe_calls: AtomicU32,
    last_world_probe: Mutex<Option<std::time::Instant>>,
    owner_thread: AtomicU32,
}
thread_local! { static LIVE: RefCell<Option<crate::lab_session::Trial>> = const {RefCell::new(None)}; }
static PROBE: OnceLock<Probe> = OnceLock::new();

fn request_event_name(nonce: &str) -> String {
    format!("WindowerRust.LabRequest.{}.{nonce}", unsafe {
        GetCurrentProcessId()
    })
}
fn accept_request(lab: &Lab) -> Result<(), String> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        nonce: String,
        process_id: u32,
        operation: String,
        character: String,
        zone: u32,
        online: bool,
        in_event: bool,
    }
    if std::fs::metadata(&lab.config.request)
        .map_err(|e| e.to_string())?
        .len()
        > 4096
    {
        return Err("Lab request exceeds limit".into());
    }
    let request: Request =
        serde_json::from_slice(&std::fs::read(&lab.config.request).map_err(|e| e.to_string())?)
            .map_err(|_| "Invalid lab request")?;
    if request.nonce != lab.config.nonce
        || request.process_id != unsafe { GetCurrentProcessId() }
        || request.character != "Mithra"
        || !matches!(request.zone, 102 | 230)
        || !request.online
        || request.in_event
    {
        return Err("Lab request identity/state mismatch".into());
    }
    if request.operation
        == lab
            .config
            .scenario
            .operation()
            .replacen("run_", "probe_", 1)
    {
        if lab.requested.load(Ordering::Acquire) || lab.world_probing.swap(true, Ordering::AcqRel) {
            return Err("World probe request already consumed".into());
        }
        return Ok(());
    }
    if request.operation != lab.config.scenario.operation()
        || !lab.world_probing.load(Ordering::Acquire)
        || lab.world_probe_calls.load(Ordering::Acquire) == 0
    {
        return Err("Lab scenario requires a matching world probe first".into());
    }
    if lab.requested.swap(true, Ordering::AcqRel) {
        return Err("Lab request already consumed".into());
    }
    Ok(())
}
fn wait_probe(probe: &Probe) -> u32 {
    let Some(lab) = &probe.lab else {
        return unsafe { WaitForSingleObject(probe.event as _, 600000) };
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    loop {
        let handles = [probe.event as _, lab.request_event as _];
        let remaining = deadline
            .saturating_duration_since(std::time::Instant::now())
            .as_millis()
            .min(600000) as u32;
        let outcome = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, remaining) };
        if outcome != 1 {
            return outcome;
        }
        if let Err(error) = accept_request(lab) {
            *probe.lab_report.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(serde_json::json!({"request_error":error}));
            probe.gate.phase.store(FAILED, Ordering::Release);
            return 0;
        }
    }
}

pub fn run(config: &GameThreadProbe, scan: &ScanReport) {
    let result = install(config, scan);
    match result {
        Ok(stopped) => {
            let probe = PROBE.get().expect("Published before hooks");
            let ready = serde_json::json!({"stage":"game_thread_probe_ready","process_id":unsafe{GetCurrentProcessId()},
                "peer_threads_quiesced":stopped,"hooks_installed":true,"game_calls":0,
                "marker":if config.lab_test.is_some(){"automatic_lab_request"}else if config.addon_echo_trial {"/echo RustAddons"} else {"/echo RustBridge"},
                "response":"Rust runtime command bridge verified",
                "addon_echo_trial":config.addon_echo_trial,
                "addons_staged":if config.addon_echo_trial {3} else {0},
                "expected_game_calls":if config.lab_test.as_ref().is_some_and(|l|l.profile_control){0}else if config.lab_test.is_some(){6}else if config.addon_echo_trial {3} else {1},
                "addon_responses":if config.addon_echo_trial {crate::addon_echo_trial::RESPONSES.to_vec()} else {vec![]},
                "lab_test":config.lab_test.is_some(),
                "lab_request_event":config.lab_test.as_ref().map(|lab|request_event_name(&lab.nonce)),
                "lab_expected_game_calls":if config.lab_test.is_some(){6}else{0},
                "limits":["Fixed diagnostic effects only; no general addon command interface","Automatic lab mode uses previously audited ABI/build plus a server-confirmed online request; it does not claim a fresh natural input marker"]});
            // No command can be dispatched until the ready report is durable.
            if save(&config.ready_report, &ready).is_err() {
                return;
            }
            probe.gate.phase.store(WAITING, Ordering::Release);
            let wait = wait_probe(probe);
            if wait != 0 {
                let _ = probe.gate.phase.compare_exchange(
                    WAITING,
                    EXPIRED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
                if probe.gate.phase.load(Ordering::Acquire) == EXECUTING {
                    // A marker claimed before the deadline may finish its one call.
                    unsafe {
                        WaitForSingleObject(probe.event as _, u32::MAX);
                    }
                }
            }
            let phase = probe.gate.phase.load(Ordering::Acquire);
            let report = serde_json::json!({"stage":"game_thread_probe_result","process_id":unsafe{GetCurrentProcessId()},
                "phase":phase,"call_returned":phase==COMPLETE,"visible_effect_verified":false,
                "draw_thread_id":probe.draw_thread.load(Ordering::Acquire),
                "marker_thread_id":probe.gate.marker_thread.load(Ordering::Acquire),
                "marker_mode":probe.gate.marker_mode.load(Ordering::Acquire),
                "draw_callbacks":probe.draw_calls.load(Ordering::Acquire),
                "natural_input_callbacks":probe.natural_inputs.load(Ordering::Acquire),
                "lab_test":config.lab_test.is_some(),
                "lab_report":probe.lab_report.lock().unwrap_or_else(|e|e.into_inner()).clone(),
                "lab_login_steps":probe.lab.as_ref().map(|lab|lab.login.completed_steps()),
                "lab_world_probe_calls":probe.lab.as_ref().map(|lab|lab.world_probe_calls.load(Ordering::Acquire)),
                "trigger":if config.lab_test.is_some(){"server_confirmed_lab_request"}else{"natural_input_marker"},
                "game_calls":probe.game_calls.load(Ordering::Acquire),
                "addon_echo_trial":config.addon_echo_trial,
                "addons_staged":if config.addon_echo_trial {3} else {0},
                "expected_game_calls":if config.lab_test.is_some(){6}else if config.addon_echo_trial {3} else {1},
                "limits":["A native return is not proof of visible echo; user confirmation is required","Hooks/core remain until process exit"]});
            let _ = save(&config.result_report, &report);
        }
        Err(error) => {
            let _ = save(
                &config.ready_report,
                &serde_json::json!({"stage":"game_thread_probe_rejected","error":error,"dispatch_enabled":false}),
            );
        }
    }
}
fn save(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut f| {
            f.write_all(&bytes)?;
            f.sync_all()
        })
        .map_err(|e| e.to_string())
}
fn install(config: &GameThreadProbe, scan: &ScanReport) -> Result<usize, String> {
    use sha2::{Digest, Sha256};
    if scan.process_id != unsafe { GetCurrentProcessId() }
        || scan.module != "FFXiMain.dll"
        || !config.game_sha256.eq_ignore_ascii_case(AUDITED_GAME_SHA256)
    {
        return Err("Game probe admits only this process and the audited installed client".into());
    }
    if config.ready_report.exists() || config.result_report.exists() {
        return Err("Probe report already exists".into());
    }
    if !scan.complete_executable_coverage || !scan.patched_candidates.is_empty() {
        return Err("Incomplete or patched game code".into());
    }
    for name in ["Hook.dll", "LuaCore.dll", "Ashita.dll"] {
        let wide: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
        if unsafe { !GetModuleHandleW(wide.as_ptr()).is_null() } {
            return Err("Original runtime present; refusing game hooks".into());
        }
    }
    let settings = crate::native::settings().ok_or("Missing launch settings")?;
    let path = Path::new(&settings.ffxi_path).join("FFXiMain.dll");
    let name: Vec<_> = "FFXiMain.dll".encode_utf16().chain(Some(0)).collect();
    let module = unsafe { GetModuleHandleW(name.as_ptr()) };
    if module as usize != scan.module_base as usize {
        return Err("Loaded game module changed".into());
    }
    let mut buffer = [0u16; 32768];
    let length =
        unsafe { GetModuleFileNameW(module, buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if length == 0 || length >= buffer.len() {
        return Err("Could not identify loaded game path".into());
    }
    use std::os::windows::ffi::OsStringExt;
    let loaded = std::path::PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length]));
    if loaded.canonicalize().map_err(|e| e.to_string())?
        != path.canonicalize().map_err(|e| e.to_string())?
    {
        return Err("Loaded game differs from the selected installation".into());
    }
    let digest = format!(
        "{:x}",
        Sha256::digest(std::fs::read(path).map_err(|e| e.to_string())?)
    );
    if !digest.eq_ignore_ascii_case(&config.game_sha256) {
        return Err("Installed game digest differs from the prepared trial".into());
    }
    let locate = |name: &str, rva: u32| -> Result<usize, String> {
        let found = scan
            .signatures
            .iter()
            .find(|s| s.name == name)
            .ok_or("Missing game entry")?;
        if found.status != "unique"
            || found.locations.len() != 1
            || found.locations[0].0 != found.locations[0].1
            || found.locations[0].1.checked_sub(scan.module_base) != Some(rva)
        {
            return Err("Game entry differs from the audited build".into());
        }
        Ok(found.locations[0].1 as usize)
    };
    // Execute fixed Lua fixtures on this worker before touching game code.
    // Refuse the whole trial if any callback/action differs from the allowlist.
    let staged_inputs = if config.addon_echo_trial {
        Some(crate::addon_echo_trial::prepare()?)
    } else {
        None
    };
    let input = unsafe {
        Patch::prepare(
            locate("input_command", 0x80dc0)?,
            input_hook as *const () as usize,
            &[0x81, 0xec, 4, 4, 0, 0],
        )
    }?;
    let draw = unsafe {
        Patch::prepare(
            locate("draw_scene", 0x74c30)?,
            draw_hook as *const () as usize,
            &[0x56, 0x8b, 0xf1, 0x8b, 0x86, 0x50, 0x0d, 0, 0],
        )
    }?;
    let lab = if let Some(lab) = &config.lab_test {
        let mut image_name = [0u16; 32768];
        let len = unsafe {
            GetModuleFileNameW(
                std::ptr::null_mut(),
                image_name.as_mut_ptr(),
                image_name.len() as u32,
            )
        } as usize;
        if len == 0 || len >= image_name.len() {
            return Err("Cannot identify lab executable".into());
        }
        let image = std::path::PathBuf::from(std::ffi::OsString::from_wide(&image_name[..len]));
        if image.file_name().and_then(|p| p.to_str()) != Some("xiloader.exe")
            || lab.request.parent() != image.parent()
            || !image.starts_with("F:/FFXI-Modding/RustLocalLab")
        {
            return Err(
                "Automatic navigation is restricted to a private xiloader lab trial".into(),
            );
        }
        let event_name: Vec<_> = request_event_name(&lab.nonce)
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let event = unsafe { CreateEventW(ptr::null(), 0, 0, event_name.as_ptr()) };
        if event.is_null() {
            return Err("Cannot create lab request signal".into());
        }
        Some(Lab {
            config: lab.clone(),
            login: crate::lab_login::Login::prepare(scan)?,
            request_event: event as usize,
            requested: std::sync::atomic::AtomicBool::new(false),
            world_probing: std::sync::atomic::AtomicBool::new(false),
            world_probe_calls: AtomicU32::new(0),
            last_world_probe: Mutex::new(None),
            owner_thread: AtomicU32::new(0),
        })
    } else {
        None
    };
    publish(input.trampoline(), draw.trampoline(), staged_inputs, lab)?;
    fixed_hooks::install(&[input, draw]).map_err(Into::into)
}
fn publish(
    input: usize,
    draw: usize,
    staged_inputs: Option<Vec<windower_rs::commands::Input>>,
    lab: Option<Lab>,
) -> Result<(), String> {
    let event = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
    if event.is_null() {
        return Err("Could not create probe completion event".into());
    }
    PROBE
        .set(Probe {
            gate: Gate::new(),
            input,
            draw,
            event: event as usize,
            draw_calls: AtomicU32::new(0),
            natural_inputs: AtomicU32::new(0),
            draw_thread: AtomicU32::new(0),
            game_calls: AtomicU32::new(0),
            staged_inputs,
            lab,
            lab_report: Mutex::new(None),
        })
        .map_err(|_| "Probe already published".into())
}
unsafe extern "C" fn input_hook(text: *const c_char, mode: i32) -> i32 {
    let Some(probe) = PROBE.get() else {
        std::process::abort();
    };
    let original: Input = unsafe { std::mem::transmute(probe.input) };
    let before_error = unsafe { GetLastError() };
    let expected_marker = if probe.staged_inputs.is_some() {
        crate::addon_echo_trial::MARKER
    } else {
        MARKER
    };
    let mut marker = [0u8; MARKER.len()];
    let mut copied = 0;
    let matched = probe.lab.is_none()
        && probe.gate.phase.load(Ordering::Acquire) == WAITING
        && unsafe {
            ReadProcessMemory(
                GetCurrentProcess(),
                text.cast(),
                marker.as_mut_ptr().cast(),
                marker.len(),
                &mut copied,
            )
        } != 0
        && copied == marker.len()
        && marker == expected_marker;
    unsafe { SetLastError(before_error) };
    // Preserve original argument bytes, mode and EAX exactly; never swallow input.
    let returned = unsafe { original(text, mode) };
    let after_error = unsafe { GetLastError() };
    probe.natural_inputs.fetch_add(1, Ordering::Relaxed);
    if matched {
        probe.gate.marker(unsafe { GetCurrentThreadId() }, mode);
    }
    unsafe { SetLastError(after_error) };
    returned
}
unsafe extern "thiscall" fn draw_hook(object: *const c_void) {
    let Some(probe) = PROBE.get() else {
        std::process::abort();
    };
    let original: Draw = unsafe { std::mem::transmute(probe.draw) };
    unsafe { original(object) };
    let last_error = unsafe { GetLastError() };
    probe.draw_calls.fetch_add(1, Ordering::Relaxed);
    let thread = unsafe { GetCurrentThreadId() };
    if let Some(lab) = &probe.lab {
        let result = std::panic::catch_unwind(|| lab_frame(probe, lab, thread));
        if !matches!(result, Ok(Ok(()))) {
            let error = match result {
                Ok(Err(error)) => error,
                _ => "Lab frame panicked".into(),
            };
            *probe.lab_report.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(serde_json::json!({"error":error}));
            probe.gate.phase.store(FAILED, Ordering::Release);
            unsafe {
                SetEvent(probe.event as _);
            }
        }
        unsafe {
            SetLastError(last_error);
        }
        return;
    }
    match probe.gate.claim(thread) {
        Some(Ok(mode)) => {
            probe.draw_thread.store(thread, Ordering::Release);
            let input: Input = unsafe { std::mem::transmute(probe.input) };
            use windower_rs::commands::Backend;
            let outcome = std::panic::catch_unwind(|| {
                let mut backend =
                    unsafe { crate::game_bridge::NativeInput::from_validated_function(input) };
                let default_input = windower_rs::commands::Input {
                    origin: windower_rs::platform::Ecosystem::Native,
                    mode,
                    bytes: RESPONSE[..RESPONSE.len() - 1].to_vec(),
                };
                let inputs = probe
                    .staged_inputs
                    .as_deref()
                    .unwrap_or(std::slice::from_ref(&default_input));
                for command in inputs {
                    backend.input(command)?;
                    probe.game_calls.fetch_add(1, Ordering::Relaxed);
                }
                Ok::<(), String>(())
            });
            probe.gate.phase.store(
                if matches!(outcome, Ok(Ok(()))) {
                    COMPLETE
                } else {
                    FAILED
                },
                Ordering::Release,
            );
            unsafe {
                SetEvent(probe.event as _);
            }
        }
        Some(Err(())) => unsafe {
            probe.draw_thread.store(thread, Ordering::Release);
            SetEvent(probe.event as _);
        },
        None => {}
    }
    unsafe { SetLastError(last_error) };
}

fn lab_frame(probe: &Probe, lab: &Lab, thread: u32) -> Result<(), String> {
    let phase = probe.gate.phase.load(Ordering::Acquire);
    if phase != WAITING && phase != EXECUTING {
        return Ok(());
    }
    let owner = lab
        .owner_thread
        .compare_exchange(0, thread, Ordering::AcqRel, Ordering::Acquire)
        .unwrap_or_else(|owner| owner);
    if owner != 0 && owner != thread {
        return Err("Local lab draw thread changed".into());
    }
    if phase == WAITING {
        lab.login.step()?;
        if lab.login.done()
            && lab.world_probing.load(Ordering::Acquire)
            && !lab.requested.load(Ordering::Acquire)
        {
            let now = std::time::Instant::now();
            let due = {
                let mut last = lab
                    .last_world_probe
                    .lock()
                    .map_err(|_| "World probe clock poisoned")?;
                if last.is_none_or(|previous| now.duration_since(previous).as_secs() >= 3) {
                    *last = Some(now);
                    true
                } else {
                    false
                }
            };
            if due {
                use windower_rs::commands::Backend;
                let input: Input = unsafe { std::mem::transmute(probe.input) };
                let mut native =
                    unsafe { crate::game_bridge::NativeInput::from_validated_function(input) };
                native.input(&windower_rs::commands::Input {
                    origin: windower_rs::platform::Ecosystem::Native,
                    mode: 1,
                    bytes: format!("/say !rustlab {} ready", lab.config.nonce).into_bytes(),
                })?;
                lab.world_probe_calls.fetch_add(1, Ordering::Relaxed);
            }
        }
        let probe_interval_ready = lab
            .last_world_probe
            .lock()
            .map_err(|_| "World probe clock poisoned")?
            .is_some_and(|last| last.elapsed() >= std::time::Duration::from_secs(3));
        if lab.login.done() && lab.requested.load(Ordering::Acquire) && probe_interval_ready {
            probe.gate.marker(thread, 1);
            match probe.gate.claim(thread) {
                Some(Ok(_)) => {}
                _ => return Err("Lab draw-thread activation failed".into()),
            }
            probe.draw_thread.store(thread, Ordering::Release);
            LIVE.with(|cell| -> Result<(), String> {
                let mut state = cell
                    .try_borrow_mut()
                    .map_err(|_| "Reentrant lab activation")?;
                if state.is_some() {
                    return Err("Lab session already active".into());
                }
                *state = Some(crate::lab_session::Trial::new(
                    &lab.config.nonce,
                    lab.config.scenario,
                    lab.config.file_profile.as_ref(),
                    lab.config.profile_control.then_some(&lab.config),
                )?);
                Ok(())
            })?;
        } else {
            return Ok(());
        }
    } else if phase != EXECUTING {
        return Ok(());
    }
    if probe.draw_thread.load(Ordering::Acquire) != thread {
        return Err("Persistent lab draw thread changed".into());
    }
    LIVE.with(|cell| -> Result<(), String> {
        let Ok(mut state) = cell.try_borrow_mut() else {
            return Ok(());
        };
        let session = state.as_mut().ok_or("Missing persistent lab state")?;
        let input: Input = unsafe { std::mem::transmute(probe.input) };
        let native = unsafe { crate::game_bridge::NativeInput::from_validated_function(input) };
        struct Count<'a> {
            native: crate::game_bridge::NativeInput,
            probe: &'a Probe,
        }
        impl windower_rs::commands::Backend for Count<'_> {
            fn input(&mut self, input: &windower_rs::commands::Input) -> Result<(), String> {
                self.native.input(input)?;
                self.probe.game_calls.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
        }
        let result = session.tick(&mut Count { native, probe })?;
        if let Some(report) = result {
            *probe.lab_report.lock().unwrap_or_else(|e| e.into_inner()) = Some(report);
            *state = None;
            probe.gate.phase.store(COMPLETE, Ordering::Release);
            unsafe {
                SetEvent(probe.event as _);
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    static INPUTS: Mutex<Vec<(Vec<u8>, i32)>> = Mutex::new(Vec::new());
    #[test]
    fn laboratory_request_checks_identity_state_opcode_and_replay() {
        let folder = std::env::temp_dir().join(format!(
            "windower-lab-request-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&folder).unwrap();
        let mut lab = Lab {
            config: windower_rs::runtime_options::LabTest {
                file_profile: None,
                profile_control: false,
                scenario: windower_rs::runtime_options::LabScenario::Persistent,
                nonce: "a".repeat(32),
                request: folder.join("request.json"),
            },
            login: crate::lab_login::Login::fixture_done(),
            request_event: 0,
            requested: std::sync::atomic::AtomicBool::new(false),
            world_probing: std::sync::atomic::AtomicBool::new(false),
            world_probe_calls: AtomicU32::new(0),
            last_world_probe: Mutex::new(None),
            owner_thread: AtomicU32::new(0),
        };
        let valid = serde_json::json!({"nonce":"a".repeat(32),"process_id":std::process::id(),"operation":"run_persistent","character":"Mithra","zone":230,"online":true,"in_event":false});
        for (field, value) in [
            ("nonce", serde_json::json!("b".repeat(32))),
            ("process_id", serde_json::json!(0)),
            ("operation", serde_json::json!("arbitrary")),
            ("zone", serde_json::json!(1)),
            ("online", serde_json::json!(false)),
            ("in_event", serde_json::json!(true)),
            ("commands", serde_json::json!([])),
        ] {
            let mut invalid = valid.clone();
            invalid[field] = value;
            std::fs::write(&lab.config.request, serde_json::to_vec(&invalid).unwrap()).unwrap();
            assert!(accept_request(&lab).is_err());
            assert!(!lab.requested.load(Ordering::Acquire));
        }
        std::fs::write(&lab.config.request, serde_json::to_vec(&valid).unwrap()).unwrap();
        assert!(accept_request(&lab).is_err());
        let mut probe_request = valid.clone();
        probe_request["operation"] = serde_json::json!("probe_persistent");
        std::fs::write(
            &lab.config.request,
            serde_json::to_vec(&probe_request).unwrap(),
        )
        .unwrap();
        accept_request(&lab).unwrap();
        assert!(accept_request(&lab).is_err());
        std::fs::write(&lab.config.request, serde_json::to_vec(&valid).unwrap()).unwrap();
        assert!(accept_request(&lab).is_err());
        lab.world_probe_calls.store(1, Ordering::Release);
        accept_request(&lab).unwrap();
        assert!(accept_request(&lab).is_err());
        lab.requested.store(false, Ordering::Release);
        let mut field = valid;
        field["zone"] = serde_json::json!(102);
        std::fs::write(&lab.config.request, serde_json::to_vec(&field).unwrap()).unwrap();
        accept_request(&lab).unwrap();
        assert!(accept_request(&lab).is_err());
        for scenario in [
            windower_rs::runtime_options::LabScenario::ReloadWindower,
            windower_rs::runtime_options::LabScenario::ReloadAshita,
            windower_rs::runtime_options::LabScenario::ReloadNative,
        ] {
            lab.config.scenario = scenario;
            lab.requested.store(false, Ordering::Release);
            std::fs::write(&lab.config.request, serde_json::to_vec(&field).unwrap()).unwrap();
            assert!(accept_request(&lab).is_err());
            assert!(!lab.requested.load(Ordering::Acquire));
            let mut request = field.clone();
            request["operation"] = serde_json::json!(scenario.operation());
            std::fs::write(&lab.config.request, serde_json::to_vec(&request).unwrap()).unwrap();
            accept_request(&lab).unwrap();
            assert!(accept_request(&lab).is_err());
        }
    }
    unsafe extern "C" fn fixture_input(text: *const c_char, mode: i32) -> i32 {
        INPUTS.lock().unwrap().push((
            unsafe { std::ffi::CStr::from_ptr(text) }
                .to_bytes()
                .to_vec(),
            mode,
        ));
        73
    }
    fn executable(bytes: &[u8]) -> usize {
        use windows_sys::Win32::System::Memory::*;
        let pointer = unsafe {
            VirtualAlloc(
                ptr::null(),
                4096,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_EXECUTE_READWRITE,
            )
        } as usize;
        assert_ne!(pointer, 0);
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer as *mut u8, bytes.len());
        }
        pointer
    }
    #[test]
    fn real_x86_hooks_preserve_cdecl_return_thiscall_object_and_one_shot_bridge() {
        hook_fixture(false);
    }
    #[test]
    fn real_x86_hooks_dispatch_three_staged_lua_echoes_once() {
        hook_fixture(true);
    }
    fn hook_fixture(addon_trial: bool) {
        let name = if addon_trial {
            "game_thread_probe::tests::real_x86_hooks_dispatch_three_staged_lua_echoes_once"
        } else {
            "game_thread_probe::tests::real_x86_hooks_preserve_cdecl_return_thiscall_object_and_one_shot_bridge"
        };
        if fixed_hooks::isolated_fixture(name) {
            return;
        }
        // Owned synthetic functions; neither the game nor any game DLL is used.
        let entry = executable(&[
            0x81, 0xec, 4, 4, 0, 0, 0x81, 0xc4, 4, 4, 0, 0, 0xe9, 0, 0, 0, 0,
        ]);
        let displacement = (fixture_input as *const () as u32).wrapping_sub((entry + 17) as u32);
        unsafe {
            std::ptr::copy_nonoverlapping(
                displacement.to_le_bytes().as_ptr(),
                (entry + 13) as *mut u8,
                4,
            );
        }
        // Draw's audited first 9 bytes load [ECX+0xd50]. Then increment an
        // object field to prove the exact object and original call survive.
        let draw_entry = executable(&[
            0x56, 0x8b, 0xf1, 0x8b, 0x86, 0x50, 0x0d, 0, 0, 0xff, 0x06, 0x5e, 0xc3,
        ]);
        let input_patch = unsafe {
            Patch::prepare(
                entry,
                input_hook as *const () as usize,
                &[0x81, 0xec, 4, 4, 0, 0],
            )
        }
        .unwrap();
        let draw_patch = unsafe {
            Patch::prepare(
                draw_entry,
                draw_hook as *const () as usize,
                &[0x56, 0x8b, 0xf1, 0x8b, 0x86, 0x50, 0x0d, 0, 0],
            )
        }
        .unwrap();
        let staged = addon_trial.then(|| crate::addon_echo_trial::prepare().unwrap());
        let expected_commands = staged.clone();
        publish(
            input_patch.trampoline(),
            draw_patch.trampoline(),
            staged,
            None,
        )
        .unwrap();
        fixed_hooks::install(&[input_patch, draw_patch]).unwrap();
        let input: Input = unsafe { std::mem::transmute(entry) };
        let draw: Draw = unsafe { std::mem::transmute(draw_entry) };
        let mut object = [0u32; 0x360];
        unsafe { draw(object.as_mut_ptr().cast()) };
        let probe = PROBE.get().unwrap();
        probe.gate.phase.store(WAITING, Ordering::Release);
        assert_eq!(unsafe { input(c"/echo ordinary".as_ptr(), 2) }, 73);
        unsafe { draw(object.as_mut_ptr().cast()) };
        assert_eq!(probe.game_calls.load(Ordering::Acquire), 0);
        let marker = if addon_trial {
            crate::addon_echo_trial::MARKER
        } else {
            MARKER
        };
        if addon_trial {
            assert_eq!(unsafe { input(MARKER.as_ptr().cast(), 2) }, 73);
            unsafe { draw(object.as_mut_ptr().cast()) };
            assert_eq!(probe.game_calls.load(Ordering::Acquire), 0);
        }
        assert_eq!(unsafe { input(marker.as_ptr().cast(), 2) }, 73);
        unsafe {
            draw(object.as_mut_ptr().cast());
            draw(object.as_mut_ptr().cast());
        }
        assert_eq!(object[0], if addon_trial { 5 } else { 4 });
        assert_eq!(
            probe.game_calls.load(Ordering::Acquire),
            if addon_trial { 3 } else { 1 }
        );
        assert_eq!(probe.gate.phase.load(Ordering::Acquire), COMPLETE);
        let mut expected = vec![(b"/echo ordinary".to_vec(), 2)];
        if addon_trial {
            expected.push((MARKER[..MARKER.len() - 1].to_vec(), 2));
        }
        expected.push((marker[..marker.len() - 1].to_vec(), 2));
        if let Some(commands) = expected_commands {
            expected.extend(
                commands
                    .into_iter()
                    .map(|command| (command.bytes, command.mode)),
            );
        } else {
            expected.push((RESPONSE[..RESPONSE.len() - 1].to_vec(), 2));
        }
        assert_eq!(*INPUTS.lock().unwrap(), expected);
        assert_eq!(
            probe.natural_inputs.load(Ordering::Acquire),
            if addon_trial { 3 } else { 2 }
        );
        assert_eq!(probe.gate.marker_thread.load(Ordering::Acquire), unsafe {
            GetCurrentThreadId()
        });
    }
    #[test]
    fn marker_requires_valid_mode_matching_thread_and_dispatches_once() {
        let gate = Gate::new();
        gate.marker(7, 1);
        assert!(gate.claim(7).is_none());
        gate.phase.store(WAITING, Ordering::Release);
        gate.marker(7, -1);
        assert!(gate.claim(7).is_none());
        gate.marker(7, 3);
        assert_eq!(gate.claim(7), Some(Ok(3)));
        assert!(gate.claim(7).is_none());
        let wrong = Gate::new();
        wrong.phase.store(WAITING, Ordering::Release);
        wrong.marker(7, 1);
        assert_eq!(wrong.claim(8), Some(Err(())));
        assert!(wrong.claim(7).is_none());
        let expired = Gate::new();
        expired.phase.store(EXPIRED, Ordering::Release);
        expired.marker(7, 1);
        assert!(expired.claim(7).is_none());
    }
}
