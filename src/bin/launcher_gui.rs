//! Native profile editor. No game process is opened or started by this editor.
#![cfg_attr(windows, windows_subsystem = "windows")]
#[cfg(windows)]
#[path = "launcher_gui/package_panel.rs"]
mod package_panel;
#[cfg(windows)]
mod app {
    use crate::package_panel;
    use std::{
        path::{Path, PathBuf},
        ptr,
        sync::mpsc::{self, Receiver, TryRecvError},
    };
    use windower_rs::{
        addon_control::Command,
        addon_profile::AddonProfile,
        editor_selection::Selection,
        launcher_document::Document,
        platform::Ecosystem,
        runtime_options::{AddonSpec, LuaEngine, NativeLibrary},
    };
    use windows_sys::Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{Input::KeyboardAndMouse::EnableWindow, WindowsAndMessaging::*},
    };
    const SAVE: usize = 101;
    const VALIDATE: usize = 102;
    const LOAD: usize = 103;
    const CAPABILITIES: usize = 104;
    const RUNTIME_STATUS: usize = 105;
    const ADDON_LOAD: usize = 106;
    const ADDON_RELOAD: usize = 107;
    const ADDON_UNLOAD: usize = 108;
    const SET_ENGINE: usize = 109;
    const PACKAGES: usize = 110;
    pub(crate) type Reply = Result<serde_json::Value, String>;
    struct Form {
        workspace: PathBuf,
        fields: Vec<HWND>,
        status: HWND,
        scale: f64,
        pending: Option<Receiver<Reply>>,
        last_reply: Option<Reply>,
        timer_ticks: u64,
        selection: Selection,
    }
    pub(crate) fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    pub(crate) fn set(hwnd: HWND, text: &str) {
        unsafe {
            SetWindowTextW(hwnd, wide(text).as_ptr());
        }
    }
    pub(crate) fn get(hwnd: HWND) -> String {
        let mut text = vec![0u16; 4097];
        let count = unsafe { GetWindowTextW(hwnd, text.as_mut_ptr(), text.len() as i32) };
        String::from_utf16_lossy(&text[..count.max(0) as usize])
    }
    impl Form {
        fn selected_engine(&self) -> LuaEngine {
            if unsafe { SendMessageW(self.fields[14], CB_GETCURSEL, 0, 0) } == 1 {
                LuaEngine::LuaJit
            } else {
                LuaEngine::Lua51
            }
        }
        fn sync_engine(&self) {
            let default = if get(self.fields[9]).trim().eq_ignore_ascii_case("luajit") {
                LuaEngine::LuaJit
            } else {
                LuaEngine::Lua51
            };
            let engine = self.selection.engine(get(self.fields[13]).trim(), default);
            unsafe {
                SendMessageW(
                    self.fields[14],
                    CB_SETCURSEL,
                    usize::from(engine == LuaEngine::LuaJit),
                    0,
                );
            }
        }
        fn runtime_action(&mut self, id: usize) -> Result<String, String> {
            if self.pending.is_some() {
                return Err("A runtime request is still pending".into());
            }
            let ready = PathBuf::from(get(self.fields[12]));
            if !ready.is_absolute() || !ready.is_file() {
                return Err("Choose the existing capture-only runtime session file".into());
            }
            let name = get(self.fields[13]).trim().to_owned();
            let command = match id {
                RUNTIME_STATUS => Command::Status {},
                ADDON_LOAD | ADDON_RELOAD => {
                    let source = self.document()?.profile.resolve(&name)?;
                    if id == ADDON_LOAD {
                        Command::Load {
                            name: source.name,
                            sha256: source.sha256,
                        }
                    } else {
                        Command::Reload {
                            name: source.name,
                            sha256: source.sha256,
                        }
                    }
                }
                ADDON_UNLOAD => Command::Unload { name },
                _ => return Err("Unknown runtime action".into()),
            };
            // Validate before spawning or publishing any request.
            windower_rs::addon_control::Request {
                schema_version: 1,
                process_id: 1,
                nonce: "a".repeat(32),
                sequence: 1,
                command: command.clone(),
            }
            .validate(1, &"a".repeat(32), 1)?;
            let (sender, receiver) = mpsc::channel();
            std::thread::Builder::new()
                .name("addon-control-request".into())
                .spawn(move || {
                    let result = windower_rs::addon_control_client::send_next(&ready, command);
                    let _ = sender.send(result);
                })
                .map_err(|e| e.to_string())?;
            self.pending = Some(receiver);
            self.last_reply = None;
            Ok("Waiting for runtime reply. The editor remains available; addon effects stay captured.".into())
        }
        fn poll_runtime(&mut self) -> Option<String> {
            let result = match self.pending.as_ref()?.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    Err("Runtime request worker exited without a result".into())
                }
            };
            self.pending = None;
            let text = match &result {
                Err(e) => format!("Runtime error: {e}. Inspect the session before retrying."),
                Ok(reply) if reply["ok"] == false => format!(
                    "Runtime rejected the request: {}",
                    reply["error"].as_str().unwrap_or("Unknown rejection")
                ),
                Ok(reply) => {
                    let names = reply["status"]["loaded_sources"]
                        .as_object()
                        .map(|m| m.keys().cloned().collect::<Vec<_>>().join(", "))
                        .unwrap_or_default();
                    format!(
                        "Runtime reply {}: loaded addons: {}. Effects captured; no game commands sent.",
                        reply["sequence"],
                        if names.is_empty() { "none" } else { &names }
                    )
                }
            };
            self.last_reply = Some(result);
            Some(text)
        }
        fn document(&self) -> Result<Document, String> {
            let mut addons = Vec::new();
            let engine = match get(self.fields[9]).trim().to_ascii_lowercase().as_str() {
                "lua51" => LuaEngine::Lua51,
                "luajit" => LuaEngine::LuaJit,
                _ => return Err("Lua engine must be lua51 or luajit".into()),
            };
            for (index, ecosystem) in [
                (6, Ecosystem::Windower4),
                (7, Ecosystem::Ashita4),
                (8, Ecosystem::Native),
            ] {
                for name in get(self.fields[index])
                    .split(',')
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                {
                    addons.push(AddonSpec {
                        name: name.into(),
                        ecosystem,
                        engine,
                    });
                }
            }
            let provider = get(self.fields[10]);
            self.selection.apply(&mut addons);
            let doc = Document {
                schema_version: 1,
                name: get(self.fields[0]),
                width: get(self.fields[2])
                    .parse()
                    .map_err(|_| "Width must be a whole number")?,
                height: get(self.fields[3])
                    .parse()
                    .map_err(|_| "Height must be a whole number")?,
                ui_scale: get(self.fields[4])
                    .parse()
                    .map_err(|_| "UI scale must be a number")?,
                profile: AddonProfile {
                    schema_version: 1,
                    root: PathBuf::from(get(self.fields[1])),
                    addons,
                    luajit_provider: if provider.is_empty() {
                        None
                    } else {
                        Some(NativeLibrary {
                            path: provider.into(),
                            sha256: get(self.fields[11]),
                        })
                    },
                },
            };
            doc.validate()?;
            Ok(doc)
        }
        fn display(&mut self, doc: &Document) {
            self.selection = Selection::from_specs(&doc.profile.addons);
            for (i, value) in [
                (0, doc.name.clone()),
                (1, doc.profile.root.display().to_string()),
                (2, doc.width.to_string()),
                (3, doc.height.to_string()),
                (4, doc.ui_scale.to_string()),
            ] {
                set(self.fields[i], &value);
            }
            for (i, e) in [
                (6, Ecosystem::Windower4),
                (7, Ecosystem::Ashita4),
                (8, Ecosystem::Native),
            ] {
                set(
                    self.fields[i],
                    &doc.profile
                        .addons
                        .iter()
                        .filter(|a| a.ecosystem == e)
                        .map(|a| a.name.clone())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
            }
            set(
                self.fields[9],
                if doc
                    .profile
                    .addons
                    .first()
                    .is_some_and(|a| a.engine == LuaEngine::LuaJit)
                {
                    "luajit"
                } else {
                    "lua51"
                },
            );
            set(
                self.fields[10],
                &doc.profile
                    .luajit_provider
                    .as_ref()
                    .map(|p| p.path.display().to_string())
                    .unwrap_or_default(),
            );
            set(
                self.fields[11],
                &doc.profile
                    .luajit_provider
                    .as_ref()
                    .map(|p| p.sha256.clone())
                    .unwrap_or_default(),
            );
            self.sync_engine();
        }
        fn action(&mut self, id: usize) -> Result<String, String> {
            match id {
                VALIDATE => { let pins = self.document()?.validate_files()?;
                    Ok(format!("Validated {} addon source(s). Display settings are saved for future launch integration.", pins.len())) }
                SAVE => { let path = self.document()?.save_new(&self.workspace)?; set(self.fields[5], &path.display().to_string());
                    Ok(format!("Saved a new profile version: {}", path.display())) }
                LOAD => { let doc = Document::read(Path::new(&get(self.fields[5])))?;
                    self.display(&doc); Ok("Loaded profile. Validate source files before saving.".into()) }
                SET_ENGINE => {
                    let name=get(self.fields[13]);let engine=self.selected_engine();
                    let mut doc=self.document()?;
                    let addon=doc.profile.addons.iter_mut().find(|a|a.name.eq_ignore_ascii_case(name.trim())).ok_or("Choose an addon selected in this profile")?;
                    addon.engine=engine;doc.validate_files()?;
                    self.selection.set(name.trim(),engine);
                    Ok(format!("{} now uses {} in this profile. Save a new version; running runtimes retain their existing engine selection.",name.trim(),if engine==LuaEngine::LuaJit{"LuaJIT"}else{"Lua 5.1"}))
                }
                PACKAGES => { package_panel::open(self.workspace.clone(), self.fields[1])?; Ok("Opened the independent local package manager.".into()) },
                CAPABILITIES => Ok("Offline editor: Windower 4, Ashita 4 and native Lua selections; Lua 5.1 or pinned LuaJIT. Live game services and third-party native plugins remain incomplete.".into()),
                RUNTIME_STATUS..=ADDON_UNLOAD => self.runtime_action(id),
                _ => Err("Unknown editor action".into()),
            }
        }
    }
    unsafe extern "system" fn procedure(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        if message == WM_COMMAND {
            let form = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Form };
            if !form.is_null() && w & 0xffff == 213 && w >> 16 == EN_CHANGE as usize {
                unsafe {
                    (*form).sync_engine();
                }
                return 0;
            }
            if !form.is_null() && w >> 16 == 0 && (101..=110).contains(&(w & 0xffff)) {
                let form = unsafe { &mut *form };
                let text = match form.action(w & 0xffff) {
                    Ok(s) => s,
                    Err(e) => format!("Error: {e}"),
                };
                set(form.status, &text);
                if form.pending.is_some() {
                    runtime_buttons(hwnd, false);
                }
                return 0;
            }
        }
        if message == WM_TIMER && w == 1 {
            let form = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Form };
            if !form.is_null() {
                let form = unsafe { &mut *form };
                form.timer_ticks += 1;
                if let Some(text) = form.poll_runtime() {
                    set(form.status, &text);
                    runtime_buttons(hwnd, true);
                }
            }
            return 0;
        }
        if message == WM_CLOSE {
            let form = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Form };
            if !form.is_null() && unsafe { (*form).pending.is_some() } {
                set(
                    unsafe { (*form).status },
                    "Wait for the pending runtime reply before closing.",
                );
                return 0;
            }
            unsafe {
                DestroyWindow(hwnd);
            }
            return 0;
        }
        if message == WM_DESTROY {
            unsafe {
                PostQuitMessage(0);
            }
            return 0;
        }
        unsafe { DefWindowProcW(hwnd, message, w, l) }
    }
    fn runtime_buttons(hwnd: HWND, enabled: bool) {
        for id in RUNTIME_STATUS..=ADDON_UNLOAD {
            unsafe {
                EnableWindow(GetDlgItem(hwnd, id as i32), i32::from(enabled));
            }
        }
    }
    pub(crate) fn control(
        parent: HWND,
        class: &str,
        text: &str,
        rect: [i32; 4],
        style: u32,
        id: usize,
        scale: f64,
    ) -> Result<HWND, String> {
        let [x, y, width, height] = rect.map(|v| (f64::from(v) * scale).round() as i32);
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                wide(class).as_ptr(),
                wide(text).as_ptr(),
                WS_CHILD | WS_VISIBLE | style,
                x,
                y,
                width,
                height,
                parent,
                id as HMENU,
                GetModuleHandleW(ptr::null()),
                ptr::null(),
            )
        };
        if hwnd.is_null() {
            return Err("Cannot create a launcher form control".into());
        }
        unsafe {
            SendMessageW(
                hwnd,
                WM_SETFONT,
                GetStockObject(DEFAULT_GUI_FONT) as usize,
                1,
            );
        }
        if class == "EDIT" {
            unsafe {
                SendMessageW(hwnd, 0x00c5, 4096, 0);
            }
        }
        Ok(hwnd)
    }
    fn build(workspace: PathBuf) -> Result<(HWND, Box<Form>), String> {
        let module = unsafe { GetModuleHandleW(ptr::null()) };
        let dc = unsafe { GetDC(ptr::null_mut()) };
        let scale = if dc.is_null() {
            1.0
        } else {
            let s = unsafe { GetDeviceCaps(dc, LOGPIXELSX as i32) } as f64 / 96.0;
            unsafe {
                ReleaseDC(ptr::null_mut(), dc);
            }
            s.max(1.0)
        };
        let class = wide("WindowerRust.OfflineLauncherEditor");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: module,
            lpszClassName: class.as_ptr(),
            hCursor: unsafe { LoadCursorW(ptr::null_mut(), IDC_ARROW) },
            hbrBackground: (COLOR_BTNFACE + 1) as HBRUSH,
            ..unsafe { std::mem::zeroed() }
        };
        if unsafe { RegisterClassW(&wc) } == 0 {
            return Err("Cannot register launcher editor".into());
        }
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_CONTROLPARENT,
                class.as_ptr(),
                wide("Signet - Profile Workbench").as_ptr(),
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                (800.0 * scale) as i32,
                (825.0 * scale) as i32,
                ptr::null_mut(),
                ptr::null_mut(),
                module,
                ptr::null(),
            )
        };
        if hwnd.is_null() {
            return Err("Cannot create launcher editor".into());
        }
        let mut form = Box::new(Form {
            workspace,
            fields: vec![],
            status: ptr::null_mut(),
            scale,
            pending: None,
            last_reply: None,
            timer_ticks: 0,
            selection: Selection::default(),
        });
        control(
            hwnd,
            "STATIC",
            "Profiles are versioned in this workspace. Addon names are comma-separated.",
            [20, 12, 620, 24],
            0,
            0,
            scale,
        )?;
        control(
            hwnd,
            "BUTTON",
            "&Packages",
            [650, 10, 105, 28],
            WS_TABSTOP,
            PACKAGES,
            scale,
        )?;
        let rows = [
            ("Profile &name", "Lab"),
            ("&Runtime root", ""),
            ("&Width", "3840"),
            ("&Height", "2160"),
            ("UI &scale", "1.5"),
            ("Saved profile &file", ""),
            ("&Windower 4 addons", ""),
            ("&Ashita 4 addons", ""),
            ("&Native addons", ""),
            ("Default Lua &engine", "lua51"),
            ("LuaJIT DLL &path", ""),
            ("LuaJIT SHA256", ""),
        ];
        for (i, (label, initial)) in rows.iter().enumerate() {
            let y = 45 + i as i32 * 35;
            control(hwnd, "STATIC", label, [20, y + 3, 160, 25], 0, 0, scale)?;
            form.fields.push(control(
                hwnd,
                "EDIT",
                initial,
                [185, y, 570, 26],
                WS_TABSTOP | WS_BORDER | 0x0080,
                200 + i,
                scale,
            )?);
        }
        for (i, label) in [
            (VALIDATE, "&Validate files"),
            (SAVE, "&Save new version"),
            (LOAD, "&Load profile"),
            (CAPABILITIES, "&Capabilities"),
        ] {
            control(
                hwnd,
                "BUTTON",
                label,
                [20 + (i as i32 - 101) * 190, 480, 180, 34],
                WS_TABSTOP,
                i,
                scale,
            )?;
        }
        for (i, label) in [(12, "Runtime session file"), (13, "Selected addon")] {
            let y = 530 + (i as i32 - 12) * 35;
            control(hwnd, "STATIC", label, [20, y + 3, 160, 25], 0, 0, scale)?;
            form.fields.push(control(
                hwnd,
                "EDIT",
                "",
                [185, y, if i == 13 { 215 } else { 570 }, 26],
                WS_TABSTOP | WS_BORDER | 0x0080,
                200 + i,
                scale,
            )?);
        }
        control(hwnd, "STATIC", "Engine", [410, 568, 50, 25], 0, 0, scale)?;
        let combo = control(
            hwnd,
            "COMBOBOX",
            "",
            [465, 565, 125, 120],
            WS_TABSTOP | 0x0003 | WS_VSCROLL,
            214,
            scale,
        )?;
        for label in ["Lua 5.1", "LuaJIT"] {
            unsafe {
                SendMessageW(combo, CB_ADDSTRING, 0, wide(label).as_ptr() as isize);
            }
        }
        unsafe {
            SendMessageW(combo, CB_SETCURSEL, 0, 0);
        }
        form.fields.push(combo);
        control(
            hwnd,
            "BUTTON",
            "Set addon engine",
            [600, 565, 155, 26],
            WS_TABSTOP,
            SET_ENGINE,
            scale,
        )?;
        for (id, label) in [
            (RUNTIME_STATUS, "Runtime status"),
            (ADDON_LOAD, "Load addon"),
            (ADDON_RELOAD, "Reload addon"),
            (ADDON_UNLOAD, "Unload addon"),
        ] {
            control(
                hwnd,
                "BUTTON",
                label,
                [20 + (id as i32 - 105) * 190, 610, 180, 34],
                WS_TABSTOP,
                id,
                scale,
            )?;
        }
        form.status = control(
            hwnd,
            "EDIT",
            "Choose a runtime root, then add selections and validate.",
            [20, 660, 735, 90],
            WS_BORDER | 0x0004 | 0x0800 | WS_VSCROLL | WS_TABSTOP,
            300,
            scale,
        )?;
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, (&mut *form as *mut Form) as _);
        }
        if unsafe { SetTimer(hwnd, 1, 30, None) } == 0 {
            return Err("Cannot create runtime reply timer".into());
        }
        Ok((hwnd, form))
    }
    pub(crate) fn capture(hwnd: HWND, path: &Path) -> Result<(), String> {
        use std::io::Write;
        if path.exists() {
            return Err("Choose a new editor capture".into());
        }
        let mut r = RECT::default();
        unsafe {
            GetClientRect(hwnd, &mut r);
        }
        let (width, height) = (r.right, r.bottom);
        let dc = unsafe { GetDC(hwnd) };
        let memory = unsafe { CreateCompatibleDC(dc) };
        let bitmap = unsafe { CreateCompatibleBitmap(dc, width, height) };
        if dc.is_null() || memory.is_null() || bitmap.is_null() {
            return Err("Cannot allocate editor capture".into());
        }
        let old = unsafe { SelectObject(memory, bitmap) };
        unsafe {
            SendMessageW(
                hwnd,
                WM_PRINT,
                memory as usize,
                (PRF_CLIENT | PRF_CHILDREN | PRF_ERASEBKGND) as isize,
            );
            SelectObject(memory, old);
        }
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: 40,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..unsafe { std::mem::zeroed() }
            },
            ..unsafe { std::mem::zeroed() }
        };
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        let lines = unsafe {
            GetDIBits(
                memory,
                bitmap,
                0,
                height as u32,
                pixels.as_mut_ptr().cast(),
                &mut info,
                DIB_RGB_COLORS,
            )
        };
        unsafe {
            DeleteObject(bitmap);
            DeleteDC(memory);
            ReleaseDC(hwnd, dc);
        }
        if lines != height {
            return Err("Editor capture readback failed".into());
        }
        let mut bytes = vec![];
        bytes.extend(b"BM");
        bytes.extend((54 + pixels.len() as u32).to_le_bytes());
        bytes.extend([0u8; 4]);
        bytes.extend(54u32.to_le_bytes());
        bytes.extend(40u32.to_le_bytes());
        bytes.extend(width.to_le_bytes());
        bytes.extend((-height).to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(32u16.to_le_bytes());
        bytes.extend([0u8; 24]);
        bytes.extend(pixels);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())
    }
    fn wait_runtime(hwnd: HWND) -> Result<serde_json::Value, String> {
        let form = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Form };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while unsafe { (*form).pending.is_some() } {
            if std::time::Instant::now() >= deadline {
                return Err("GUI runtime reply deadline expired".into());
            }
            let mut message = MSG::default();
            while unsafe { PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
                unsafe {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        unsafe { (*form).last_reply.clone() }.ok_or("GUI returned no runtime result")?
    }
    fn control_test(hwnd: HWND, args: &[String]) -> Result<(), String> {
        let document = Document::read(Path::new(&args[2]))?;
        let form = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Form };
        unsafe {
            (*form).display(&document);
        }
        set(unsafe { (&(*form).fields)[12] }, &args[3]);
        let click = |id| unsafe { SendMessageW(hwnd, WM_COMMAND, id, 0) };
        let mut replies = Vec::new();
        for (id, name, accepted, count) in [
            (RUNTIME_STATUS, "", true, 0),
            (ADDON_LOAD, "selected", true, 1),
            (ADDON_LOAD, "neighbor", true, 2),
            (ADDON_LOAD, "selected", false, 2),
            (ADDON_RELOAD, "selected", true, 2),
            (ADDON_UNLOAD, "selected", true, 1),
            (ADDON_LOAD, "selected", true, 2),
            (ADDON_UNLOAD, "neighbor", true, 1),
            (ADDON_UNLOAD, "selected", true, 0),
            (RUNTIME_STATUS, "", true, 0),
        ] {
            set(unsafe { (&(*form).fields)[13] }, name);
            click(id);
            if !unsafe { (*form).pending.is_some() } {
                return Err("GUI did not start asynchronous runtime request".into());
            }
            click(CAPABILITIES);
            if !get(unsafe { (*form).status }).starts_with("Offline editor:") {
                return Err("GUI did not respond while a runtime request was pending".into());
            }
            let result = wait_runtime(hwnd)?;
            if result["ok"] != accepted
                || result["status"]["loaded_sources"]
                    .as_object()
                    .map(|m| m.len())
                    != Some(count)
                || result["game_commands_sent"] != 0
            {
                return Err(format!("GUI runtime state differs: {result}"));
            }
            replies.push(result);
        }
        // Frontend errors must not publish any transport request.
        let sequence = replies.last().unwrap()["sequence"].as_u64().unwrap();
        set(unsafe { (&(*form).fields)[13] }, "missing-addon");
        click(ADDON_RELOAD);
        if unsafe { (*form).pending.is_some() }
            || !get(unsafe { (*form).status }).starts_with("Error:")
        {
            return Err("GUI accepted an unselected addon".into());
        }
        set(unsafe { (&(*form).fields)[12] }, "");
        click(RUNTIME_STATUS);
        if unsafe { (*form).pending.is_some() }
            || !get(unsafe { (*form).status }).starts_with("Error:")
        {
            return Err("GUI accepted a missing runtime session".into());
        }
        set(unsafe { (&(*form).fields)[12] }, &args[3]);
        set(unsafe { (&(*form).fields)[13] }, "selected");
        set(
            unsafe { (*form).status },
            "Runtime lifecycle verified in an isolated host. No addons loaded; no game commands sent.",
        );
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
        capture(hwnd, Path::new(&args[5]))?;
        let report = serde_json::json!({"checks_passed":true,"replies":replies,"next_sequence":sequence+1,
            "responsive_during_request":true,"frontend_rejections":2,"runtime_rejections":1,
            "timer_ticks":unsafe{(*form).timer_ticks},"offscreen_owned_window":true,"game_started":false,"game_commands_sent":0});
        use std::io::Write;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args[4])
            .and_then(|mut f| {
                f.write_all(serde_json::to_string_pretty(&report).unwrap().as_bytes())
            })
            .map_err(|e| e.to_string())?;
        unsafe {
            DestroyWindow(hwnd);
        }
        Ok(())
    }
    fn engine_test(hwnd: HWND, args: &[String]) -> Result<(), String> {
        let original = Document::read(Path::new(&args[2]))?;
        let form = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Form };
        unsafe {
            (*form).display(&original);
        }
        let click = |id| unsafe { SendMessageW(GetDlgItem(hwnd, id as i32), BM_CLICK, 0, 0) };
        let saved = || -> Result<(PathBuf, Document), String> {
            click(SAVE);
            if get(unsafe { (*form).status }).starts_with("Error:") {
                return Err(get(unsafe { (*form).status }));
            }
            let path = PathBuf::from(get(unsafe { (&(*form).fields)[5] }));
            let doc = Document::read(&path)?;
            Ok((path, doc))
        };
        let (first, roundtrip) = saved()?;
        if roundtrip.fingerprint()? != original.fingerprint()? {
            return Err("Mixed-engine profile changed during GUI roundtrip".into());
        }
        set(unsafe { (&(*form).fields)[13] }, "selected");
        if unsafe { (*form).selected_engine() } != LuaEngine::LuaJit {
            return Err("GUI did not show selected addon engine".into());
        }
        unsafe {
            SendMessageW((&(*form).fields)[14], CB_SETCURSEL, 0, 0);
        }
        click(SET_ENGINE);
        let (second, changed) = saved()?;
        let selected = changed
            .profile
            .addons
            .iter()
            .find(|a| a.name == "selected")
            .ok_or("Missing selected addon")?;
        if selected.engine != LuaEngine::Lua51
            || changed.profile.addons[0] != original.profile.addons[0]
        {
            return Err("GUI engine change affected the wrong addon/order".into());
        }
        unsafe {
            SendMessageW((&(*form).fields)[14], CB_SETCURSEL, 1, 0);
        }
        click(SET_ENGINE);
        let (third, restored) = saved()?;
        if restored.fingerprint()? != original.fingerprint()? {
            return Err("Selected engine restore changed its neighbor/order".into());
        }
        set(unsafe { (&(*form).fields)[13] }, "not-selected");
        click(SET_ENGINE);
        if !get(unsafe { (*form).status }).starts_with("Error:") {
            return Err("GUI accepted an unselected engine edit".into());
        }
        set(unsafe { (&(*form).fields)[13] }, "selected");
        let provider_pin = get(unsafe { (&(*form).fields)[11] });
        set(unsafe { (&(*form).fields)[11] }, &"0".repeat(64));
        click(VALIDATE);
        if !get(unsafe { (*form).status }).starts_with("Error:") {
            return Err("GUI accepted wrong provider bytes".into());
        }
        set(unsafe { (&(*form).fields)[11] }, &provider_pin);
        click(VALIDATE);
        if !get(unsafe { (*form).status }).starts_with("Validated 2") {
            return Err("GUI failed to restore valid provider pin".into());
        }
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
        let report = serde_json::json!({"checks_passed":true,"profiles":[first,second,third],"roundtrip_preserved":true,
            "selected_only_change":true,"order_preserved":true,"provider_pin_rejected":true,"unselected_rejected":true,
            "native_buttons_clicked":true,"offscreen_owned_window":true,"game_started":false});
        use std::io::Write;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args[3])
            .and_then(|mut f| {
                f.write_all(serde_json::to_string_pretty(&report).unwrap().as_bytes())
            })
            .map_err(|e| e.to_string())?;
        unsafe {
            DestroyWindow(hwnd);
        }
        Ok(())
    }
    pub fn run() -> Result<(), String> {
        let args: Vec<_> = std::env::args().skip(1).collect();
        if args.first().is_some_and(|a| a == "--package-test") {
            if args.len() != 5 || Path::new(&args[3]).exists() || Path::new(&args[4]).exists() {
                return Err("Choose fresh package test evidence paths".into());
            }
            let scenario: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&args[2]).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let original = Document::read(Path::new(
                scenario["editor_profile"]
                    .as_str()
                    .ok_or("Missing package editor fixture")?,
            ))?;
            let (hwnd, mut form) = build(PathBuf::from(&args[1]))?;
            form.display(&original);
            package_panel::test(&args, form.fields[1])?;
            unsafe {
                SendMessageW(GetDlgItem(hwnd, SAVE as i32), BM_CLICK, 0, 0);
            }
            let path = PathBuf::from(get(form.fields[5]));
            let saved = Document::read(&path)?;
            if saved.profile.root.as_path() != Path::new(&get(form.fields[1]))
                || saved.profile.addons != original.profile.addons
            {
                return Err("Prepared package root did not save through the native editor".into());
            }
            let receipt = serde_json::json!({"checks_passed":true,"native_save_clicked":true,"document":path,"root":saved.profile.root,"fingerprint":saved.fingerprint()?});
            use std::io::Write;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(Path::new(&args[3]).with_extension("saved-profile.json"))
                .and_then(|mut f| {
                    f.write_all(serde_json::to_string_pretty(&receipt).unwrap().as_bytes())
                })
                .map_err(|e| e.to_string())?;
            unsafe {
                DestroyWindow(hwnd);
            }
            return Ok(());
        }
        let control_testing = args.first().is_some_and(|a| a == "--control-test");
        let engine_testing = args.first().is_some_and(|a| a == "--engine-test");
        let testing =
            args.first().is_some_and(|a| a == "--self-test") || control_testing || engine_testing;
        if (!testing && !(1..=2).contains(&args.len()))
            || (testing && args.len() != if control_testing { 6 } else { 5 })
        {
            return Err("Usage: launcher_gui WORKSPACE [SAVED-PROFILE] | --self-test WORKSPACE RUNTIME-ROOT NEW-REPORT NEW-BMP".into());
        }
        let workspace = PathBuf::from(&args[usize::from(testing)]);
        if !workspace.is_absolute() || !workspace.is_dir() {
            return Err("Choose an existing absolute workspace".into());
        }
        if testing
            && !control_testing
            && (Path::new(&args[3]).exists() || Path::new(&args[4]).exists())
        {
            return Err("Choose fresh editor evidence paths".into());
        }
        let (hwnd, mut form) = build(workspace)?;
        if engine_testing {
            return engine_test(hwnd, &args);
        }
        if control_testing {
            if Path::new(&args[4]).exists() || Path::new(&args[5]).exists() {
                return Err("Choose fresh GUI control evidence paths".into());
            }
            return control_test(hwnd, &args);
        }
        if !testing && args.len() == 2 {
            let doc = Document::read(Path::new(&args[1]))?;
            form.display(&doc);
            set(form.fields[5], &args[1]);
        }
        if testing {
            let click = |id| unsafe {
                SendMessageW(hwnd, WM_COMMAND, id, 0);
            };
            set(form.fields[1], &args[2]);
            set(form.fields[8], "NativeFixture");
            set(form.fields[2], "bad");
            click(SAVE);
            if !get(form.status).starts_with("Error:") {
                return Err("GUI accepted invalid width".into());
            }
            set(form.fields[2], "3840");
            click(VALIDATE);
            if !get(form.status).starts_with("Validated 1") {
                return Err(format!(
                    "GUI source validation failed: {}",
                    get(form.status)
                ));
            }
            click(SAVE);
            let first = PathBuf::from(get(form.fields[5]));
            let doc = Document::read(&first)?;
            set(form.fields[2], "1280");
            click(LOAD);
            if get(form.fields[2]) != "3840" {
                return Err("GUI did not restore saved display settings".into());
            }
            click(SAVE);
            let second = PathBuf::from(get(form.fields[5]));
            if first == second || Document::read(&second)?.fingerprint()? != doc.fingerprint()? {
                return Err("GUI versioned roundtrip failed".into());
            }
            click(CAPABILITIES);
            // Single-line native EDIT controls do not render fully while their
            // parent is invisible. Paint an owned offscreen window without
            // activation, then destroy it; never put a test form on the desktop.
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
            let report = serde_json::json!({"checks_passed":true,"offscreen_owned_window":true,"form_controls":form.fields.len(),"invalid_width_rejected":true,
                "source_validated":true,"saved_roundtrip":true,"prior_version_preserved":first.is_file(),"first":first,"second":second,
                "dpi_scale":form.scale,"capture":args[4],"game_started":false,"live_control_actions":"NOT_RUN in the configuration-only scenario"});
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&args[3])
                .and_then(|mut f| {
                    std::io::Write::write_all(
                        &mut f,
                        serde_json::to_string_pretty(&report).unwrap().as_bytes(),
                    )
                })
                .map_err(|e| e.to_string())?;
            unsafe {
                DestroyWindow(hwnd);
            }
            return Ok(());
        }
        unsafe {
            ShowWindow(hwnd, SW_SHOW);
        }
        let mut message = MSG::default();
        loop {
            let next = unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) };
            if next == 0 {
                break;
            }
            if next < 0 {
                return Err("Launcher message loop failed".into());
            }
            if unsafe { IsDialogMessageW(GetAncestor(message.hwnd, GA_ROOT), &message) } == 0 {
                unsafe {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        }
        drop(form);
        Ok(())
    }
}
fn main() {
    #[cfg(windows)]
    if let Err(e) = app::run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
    #[cfg(not(windows))]
    {
        eprintln!("Native launcher editor requires Windows");
        std::process::exit(1);
    }
}
