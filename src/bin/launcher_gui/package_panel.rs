//! Independent native package window; all mutations stay in WORKSPACE/packages.
use crate::app::{Reply, capture, control, get, set, wide};
use std::{
    path::{Path, PathBuf},
    ptr,
    sync::mpsc::{self, Receiver, TryRecvError},
};
use windower_rs::package_editor::{Request, execute};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Input::KeyboardAndMouse::EnableWindow, WindowsAndMessaging::*},
};
const INSPECT: usize = 401;
const STAGE: usize = 402;
const CHECK: usize = 403;
const ACTIVATE: usize = 404;
const ROLLBACK: usize = 405;
const PREPARE: usize = 406;
struct Panel {
    workspace: PathBuf,
    profile_root: HWND,
    fields: Vec<HWND>,
    status: HWND,
    pending: Option<Receiver<Reply>>,
    last: Option<Reply>,
    ticks: u64,
}
impl Panel {
    fn start(&mut self, id: usize) -> Result<(), String> {
        if self.pending.is_some() {
            return Err("A package operation is still pending".into());
        }
        let slots = || {
            get(self.fields[3])
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        };
        let request = match id {
            INSPECT => Request::Inspect,
            PREPARE => Request::Prepare,
            STAGE => Request::Stage {
                manifest: get(self.fields[1]).into(),
                source: get(self.fields[2]).into(),
            },
            CHECK => Request::Check { slots: slots() },
            ACTIVATE => Request::Activate { slots: slots() },
            ROLLBACK => Request::Rollback {
                generation: get(self.fields[4])
                    .trim()
                    .parse()
                    .map_err(|_| "Generation must be a whole number")?,
            },
            _ => return Err("Unknown package action".into()),
        };
        let workspace = self.workspace.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("package-operation".into())
            .spawn(move || {
                let _ = tx.send(execute(&workspace, request));
            })
            .map_err(|e| e.to_string())?;
        self.pending = Some(rx);
        self.last = None;
        Ok(())
    }
    fn poll(&mut self) -> Option<String> {
        let reply = match self.pending.as_ref()?.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("Package worker exited without result".into()),
        };
        self.pending = None;
        let message = match &reply {
            Ok(r) => {
                if let Some(slot) = r["receipt"]["slot"].as_str() {
                    set(self.fields[3], slot);
                }
                if let Some(root) = r["mount"]["root"].as_str()
                    && !self.profile_root.is_null()
                {
                    set(self.profile_root, root);
                }
                let current = if r["action"] == "prepare" {
                    &r["mount"]["generation"]
                } else {
                    &r["current"]
                };
                let active = current["slots"]
                    .as_object()
                    .map(|m| m.keys().cloned().collect::<Vec<_>>().join(", "))
                    .unwrap_or_default();
                let generation = current["number"]
                    .as_u64()
                    .map(|n| format!("Active generation: {n}.\r\nPackages: {active}."))
                    .unwrap_or_else(|| "No package selection is active yet.".into());
                let detail=match r["action"].as_str(){
                    Some("stage")=>format!("Staged {} {} and verified its files. Select all required versions before activation.",r["receipt"]["manifest"]["id"].as_str().unwrap_or("package"),r["receipt"]["manifest"]["version"].as_str().unwrap_or("")),
                    Some("check")=>"Files and dependencies verified. The selection is ready to activate.".into(),
                    Some("activate")=>"Selection activated. Prepare addons to use it in a new profile version.".into(),
                    Some("rollback")=>"Earlier selection restored. Existing prepared profiles keep their own files.".into(),
                    Some("prepare")=>"Addon files prepared and the profile runtime root updated. Save a new profile version to use them.".into(),
                    _=>"Current selection verified. Previous versions are retained.".into(),
                };
                format!("{detail}\r\n\r\n{generation}")
            }
            Err(e) => format!("Error: {e}"),
        };
        self.last = Some(reply);
        Some(message)
    }
}
fn enable(hwnd: HWND, yes: bool) {
    for id in INSPECT..=PREPARE {
        unsafe {
            EnableWindow(GetDlgItem(hwnd, id as i32), i32::from(yes));
        }
    }
}
unsafe extern "system" fn procedure(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let form = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Panel };
    if !form.is_null() {
        let p = unsafe { &mut *form };
        if message == WM_COMMAND && w >> 16 == 0 && (INSPECT..=PREPARE).contains(&(w & 0xffff)) {
            let text = match p.start(w & 0xffff) {
                Ok(()) => "Working; this window remains responsive.".into(),
                Err(e) => format!("Error: {e}"),
            };
            set(p.status, &text);
            enable(hwnd, p.pending.is_none());
            return 0;
        }
        if message == WM_TIMER {
            p.ticks += 1;
            if let Some(text) = p.poll() {
                set(p.status, &text);
                enable(hwnd, true);
            }
            return 0;
        }
        if message == WM_CLOSE {
            if p.pending.is_some() {
                set(
                    p.status,
                    "Wait for the package operation to finish before closing.",
                );
                return 0;
            }
            unsafe {
                DestroyWindow(hwnd);
            }
            return 0;
        }
        if message == WM_NCDESTROY {
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(form));
            }
        }
    }
    unsafe { DefWindowProcW(hwnd, message, w, l) }
}
fn build(workspace: PathBuf, profile_root: HWND) -> Result<HWND, String> {
    let class = wide("WindowerRust.PackagePanel");
    let module = unsafe { GetModuleHandleW(ptr::null()) };
    let wc = WNDCLASSW {
        lpfnWndProc: Some(procedure),
        hInstance: module,
        lpszClassName: class.as_ptr(),
        hCursor: unsafe { LoadCursorW(ptr::null_mut(), IDC_ARROW) },
        hbrBackground: (COLOR_BTNFACE + 1) as HBRUSH,
        ..unsafe { std::mem::zeroed() }
    };
    // Reopening the panel reuses the registered class.
    unsafe {
        RegisterClassW(&wc);
    }
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_CONTROLPARENT,
            class.as_ptr(),
            wide("Rust FFXI - Local Packages").as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            800,
            680,
            if profile_root.is_null() {
                ptr::null_mut()
            } else {
                GetParent(profile_root)
            },
            ptr::null_mut(),
            module,
            ptr::null(),
        )
    };
    if hwnd.is_null() {
        return Err("Cannot create package window".into());
    }
    let mut p = Box::new(Panel {
        workspace,
        profile_root,
        fields: vec![],
        status: ptr::null_mut(),
        pending: None,
        last: None,
        ticks: 0,
    });
    control(
        hwnd,
        "STATIC",
        "Stage verified local files, check dependencies, then activate. Old versions are retained.",
        [20, 15, 745, 28],
        0,
        0,
        1.0,
    )?;
    for (i, (label, initial)) in [
        (
            "Workspace store",
            p.workspace.join("packages").display().to_string(),
        ),
        ("Manifest JSON", String::new()),
        ("Source folder", String::new()),
        ("Complete slots (CSV)", String::new()),
        ("Restore generation", String::new()),
    ]
    .iter()
    .enumerate()
    {
        let y = 55 + i as i32 * 40;
        control(hwnd, "STATIC", label, [20, y + 3, 155, 25], 0, 0, 1.0)?;
        p.fields.push(control(
            hwnd,
            "EDIT",
            initial,
            [180, y, 570, 27],
            WS_TABSTOP | WS_BORDER | 0x0080 | if i == 0 { 0x0800 } else { 0 },
            500 + i,
            1.0,
        )?);
    }
    for (id, label) in [
        (INSPECT, "&Inspect"),
        (STAGE, "&Stage"),
        (CHECK, "&Check selection"),
        (ACTIVATE, "&Activate"),
        (ROLLBACK, "&Restore generation"),
        (PREPARE, "&Prepare addons"),
    ] {
        control(
            hwnd,
            "BUTTON",
            label,
            [20 + (id as i32 - 401) * 125, 270, 120, 34],
            WS_TABSTOP,
            id,
            1.0,
        )?;
    }
    control(
        hwnd,
        "STATIC",
        "Activate the complete selection, then prepare addon files for a new profile version.",
        [20, 320, 745, 32],
        0,
        0,
        1.0,
    )?;
    p.status = control(
        hwnd,
        "EDIT",
        "Choose a pinned local manifest and its source folder.",
        [20, 365, 730, 240],
        WS_BORDER | 0x0004 | 0x0800 | WS_VSCROLL | WS_TABSTOP,
        600,
        1.0,
    )?;
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(p) as _);
        SetTimer(hwnd, 1, 30, None);
    }
    Ok(hwnd)
}
pub(crate) fn open(workspace: PathBuf, profile_root: HWND) -> Result<(), String> {
    let hwnd = build(workspace, profile_root)?;
    unsafe {
        ShowWindow(hwnd, SW_SHOW);
    }
    Ok(())
}
fn pump() {
    let mut msg = MSG::default();
    unsafe {
        while PeekMessageW(&mut msg, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            if IsDialogMessageW(msg.hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}
pub(crate) fn test(args: &[String], profile_root: HWND) -> Result<(), String> {
    use std::io::Write;
    let scenario: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&args[2]).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let hwnd = build(PathBuf::from(&args[1]), profile_root)?;
    let p = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Panel };
    let click = |id: usize, ok: bool| -> Result<serde_json::Value, String> {
        unsafe {
            SendMessageW(GetDlgItem(hwnd, id as i32), BM_CLICK, 0, 0);
        }
        let pending = unsafe { (*p).pending.is_some() };
        if pending {
            if unsafe { (*p).start(INSPECT) }.is_ok() {
                return Err("Concurrent package request accepted".into());
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while unsafe { (*p).pending.is_some() } {
                if std::time::Instant::now() > deadline {
                    return Err("Package panel timeout".into());
                }
                pump();
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        let reply = unsafe { (*p).last.clone() }.ok_or("Missing package reply")?;
        if reply.is_ok() != ok {
            return Err(format!("Unexpected package result: {reply:?}"));
        }
        Ok(match reply {
            Ok(v) => v,
            Err(e) => serde_json::json!({"error":e}),
        })
    };
    let mut replies = vec![click(INSPECT, true)?];
    let mut slots = vec![];
    for item in scenario["packages"]
        .as_array()
        .ok_or("Missing package fixtures")?
    {
        set(
            unsafe { (&(*p).fields)[1] },
            item["manifest"].as_str().ok_or("Missing manifest")?,
        );
        set(
            unsafe { (&(*p).fields)[2] },
            item["source"].as_str().ok_or("Missing source")?,
        );
        let reply = click(STAGE, true)?;
        slots.push(
            reply["receipt"]["slot"]
                .as_str()
                .ok_or("Missing staged slot")?
                .to_string(),
        );
        replies.push(reply);
    }
    if slots.len() != 3 {
        return Err("Expected dependency and two addon versions".into());
    }
    set(unsafe { (&(*p).fields)[3] }, &slots[1]);
    replies.push(click(CHECK, false)?);
    replies.push(click(ACTIVATE, false)?);
    set(unsafe { (&(*p).fields)[3] }, &slots[..2].join(", "));
    replies.push(click(CHECK, true)?);
    replies.push(click(ACTIVATE, true)?);
    set(
        unsafe { (&(*p).fields)[3] },
        &[slots[0].clone(), slots[2].clone()].join(", "),
    );
    replies.push(click(ACTIVATE, true)?);
    replies.push(click(PREPARE, true)?);
    set(unsafe { (&(*p).fields)[4] }, "1");
    let restored = click(ROLLBACK, true)?;
    if restored["current"]["number"] != 3
        || restored["current"]["previous"] != 2
        || restored["current"]["slots"] != replies[7]["current"]["slots"]
    {
        return Err("Package generation restore mismatch".into());
    }
    replies.push(restored);
    let before =
        std::fs::read(Path::new(&args[1]).join("packages/CURRENT")).map_err(|e| e.to_string())?;
    set(unsafe { (&(*p).fields)[4] }, "9999");
    replies.push(click(ROLLBACK, false)?);
    set(
        unsafe { (&(*p).fields)[1] },
        scenario["bad_manifest"]
            .as_str()
            .ok_or("Missing bad manifest")?,
    );
    replies.push(click(STAGE, false)?);
    if std::fs::read(Path::new(&args[1]).join("packages/CURRENT")).map_err(|e| e.to_string())?
        != before
    {
        return Err("Rejected operation changed current generation".into());
    }
    set(unsafe { (&(*p).fields)[3] }, &slots[..2].join(", "));
    replies.push(click(INSPECT, true)?);
    let first = click(PREPARE, true)?;
    if get(profile_root)
        != first["mount"]["root"]
            .as_str()
            .ok_or("Missing prepared root")?
    {
        return Err("Prepared addon root did not reach the profile field".into());
    }
    let second = click(PREPARE, true)?;
    if first["mount"]["root"] == second["mount"]["root"]
        || !Path::new(first["mount"]["root"].as_str().unwrap())
            .join(".rust-mount.json")
            .is_file()
    {
        return Err("Preparing again replaced prior profile files".into());
    }
    replies.push(first);
    replies.push(second);
    unsafe {
        SetWindowPos(
            hwnd,
            ptr::null_mut(),
            -32000,
            -32000,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        RedrawWindow(
            hwnd,
            ptr::null(),
            ptr::null_mut(),
            RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_UPDATENOW,
        );
    }
    capture(hwnd, Path::new(&args[4]))?;
    let report = serde_json::json!({"checks_passed":true,"native_buttons_clicked":true,"offscreen_owned_window":true,"concurrent_requests_rejected":true,"timer_ticks":unsafe{(*p).ticks},"replies":replies,"staged_slots":slots,"profile_root_updated":true,"prepared_roots_preserved":true,"failed_operations_preserve_current":true,"game_started":false});
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[3])
        .and_then(|mut f| f.write_all(serde_json::to_string_pretty(&report).unwrap().as_bytes()))
        .map_err(|e| e.to_string())?;
    unsafe {
        DestroyWindow(hwnd);
    }
    Ok(())
}
