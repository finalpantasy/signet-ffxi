//! Exercise remapping with messages sent only to an owned hidden window.
#[cfg(windows)]
fn run() -> Result<(), String> {
    use std::{ptr, sync::Mutex};
    use windower_rs::bindings::{Binding, Bindings, Modifiers, Trigger};
    use windows_sys::Win32::{
        Foundation::*, System::LibraryLoader::GetModuleHandleW, UI::WindowsAndMessaging::*,
    };
    #[derive(Default)]
    struct State {
        bindings: Bindings,
        ctrl: bool,
        triggers: Vec<Trigger>,
        forwarded: usize,
    }
    static STATE: Mutex<Option<State>> = Mutex::new(None);
    unsafe extern "system" fn procedure(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        if let Ok(mut guard) = STATE.lock()
            && let Some(state) = guard.as_mut()
        {
            match message {
                WM_SETFOCUS => state.bindings.focus(true),
                WM_KILLFOCUS => {
                    state.bindings.focus(false);
                    state.ctrl = false;
                }
                WM_KEYDOWN | WM_KEYUP => {
                    let down = message == WM_KEYDOWN;
                    if w == 17 {
                        state.ctrl = down;
                    }
                    let modifiers = Modifiers {
                        ctrl: state.ctrl,
                        ..Modifiers::default()
                    };
                    let blocked = state.bindings.is_blocking(w as u16, modifiers);
                    if let Some(trigger) = state.bindings.key(w as u16, down, modifiers) {
                        state.triggers.push(trigger);
                    }
                    if blocked {
                        return 1;
                    }
                    state.forwarded += 1;
                }
                _ => {}
            }
        }
        unsafe { DefWindowProcW(hwnd, message, w, l) }
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 1 {
        return Err("Usage: bindings-probe NEW-REPORT.json".into());
    }
    if std::path::Path::new(&args[0]).exists() {
        return Err("Choose new bindings evidence".into());
    }
    let class: Vec<_> = "RustOwnedBindingsFixture"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let module = unsafe { GetModuleHandleW(ptr::null()) };
    let wc = WNDCLASSW {
        hInstance: module,
        lpszClassName: class.as_ptr(),
        lpfnWndProc: Some(procedure),
        ..unsafe { std::mem::zeroed() }
    };
    if unsafe { RegisterClassW(&wc) } == 0 {
        return Err("Cannot register owned input fixture".into());
    }
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            320,
            240,
            ptr::null_mut(),
            ptr::null_mut(),
            module,
            ptr::null(),
        )
    };
    if hwnd.is_null() {
        return Err("Cannot create owned input fixture".into());
    }
    let mut state = State::default();
    state.bindings.set(vec![Binding {
        key: 120,
        modifiers: Modifiers {
            ctrl: true,
            ..Modifiers::default()
        },
        command: "diagnostic test".into(),
        block: true,
    }])?;
    *STATE.lock().map_err(|_| "Input fixture poisoned")? = Some(state);
    let send = |m, w| unsafe { SendMessageW(hwnd, m, w, 0) };
    send(WM_SETFOCUS, 0);
    send(WM_KEYDOWN, 120);
    send(WM_KEYUP, 120); // wrong modifiers
    send(WM_KEYDOWN, 17);
    if send(WM_KEYDOWN, 120) != 1 {
        return Err("Owned window did not block matching chord".into());
    }
    if send(WM_KEYDOWN, 120) != 1 {
        return Err("Owned window forwarded a blocked repeat".into());
    }
    send(WM_KILLFOCUS, 0);
    send(WM_KEYDOWN, 120);
    send(WM_SETFOCUS, 0);
    send(WM_KEYDOWN, 17);
    send(WM_KEYDOWN, 120);
    send(WM_KEYUP, 120);
    let mut state = STATE
        .lock()
        .map_err(|_| "Input fixture poisoned")?
        .take()
        .ok_or("Missing input state")?;
    state.bindings.set(vec![Binding {
        key: 121,
        modifiers: Modifiers::default(),
        command: "remapped".into(),
        block: false,
    }])?;
    *STATE.lock().map_err(|_| "Input fixture poisoned")? = Some(state);
    send(WM_KEYUP, 17);
    send(WM_KEYDOWN, 120);
    send(WM_KEYDOWN, 121);
    let state = STATE
        .lock()
        .map_err(|_| "Input fixture poisoned")?
        .take()
        .ok_or("Missing input state")?;
    unsafe {
        DestroyWindow(hwnd);
    }
    if state.triggers.len() != 3
        || state.triggers[2].command != "remapped"
        || state.triggers[2].blocked
    {
        return Err("Repeat, focus-loss or remapping behavior differs".into());
    }
    let report = serde_json::json!({"checks_passed":true,"hidden_owned_window":true,"triggers":state.triggers,
        "forwarded_key_messages":state.forwarded,"repeat_suppressed":true,"focus_loss_clears_held":true,"game_input_sent":0});
    use std::io::Write;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[0])
        .and_then(|mut f| f.write_all(serde_json::to_string_pretty(&report).unwrap().as_bytes()))
        .map_err(|e| e.to_string())
}
fn main() {
    #[cfg(windows)]
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
    #[cfg(not(windows))]
    {
        eprintln!("Requires Windows");
        std::process::exit(1);
    }
}
