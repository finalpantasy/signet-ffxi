//! Read-only access to a running retail client (Windows only).
//!
//! What this module does: find the process that has FFXiMain.dll loaded, open it with
//! PROCESS_QUERY_LIMITED_INFORMATION and PROCESS_VM_READ only, copy the module image once to
//! resolve signatures, then read a few small blocks per sample with ReadProcessMemory.
//!
//! What it never does: write to the process, inject, hook, attach a debugger, send packets or
//! synthesize input. Keys are read with GetAsyncKeyState. The mouse wheel and relative mouse
//! counts come from a Raw Input registration on this probe's own hidden window (RIDEV_INPUTSINK),
//! which only receives a copy of the device input; no hook is installed in any process.

use crate::layout::Layout;
use crate::rows::*;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows::Win32::System::Diagnostics::ToolHelp::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ};
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::Input::{GetRawInputData, HRAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RIDEV_INPUTSINK, RID_INPUT, RegisterRawInputDevices};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

pub struct Process {
    pub pid: u32,
    pub base: u32,
    pub size: u32,
    pub exe: String,
    handle: HANDLE,
}

impl Drop for Process {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

fn wstr(s: &[u16]) -> String {
    let n = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    String::from_utf16_lossy(&s[..n])
}

/// Processes that have FFXiMain.dll loaded: (pid, exe name, module base, module size).
pub fn find_clients() -> Result<Vec<(u32, String, u32, u32)>, String> {
    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).map_err(|e| format!("process snapshot: {e}"))?;
        let mut pe = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut pe).is_ok();
        while ok {
            let (pid, exe) = (pe.th32ProcessID, wstr(&pe.szExeFile));
            if let Ok(ms) = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) {
                let mut me = MODULEENTRY32W { dwSize: std::mem::size_of::<MODULEENTRY32W>() as u32, ..Default::default() };
                let mut mok = Module32FirstW(ms, &mut me).is_ok();
                while mok {
                    if wstr(&me.szModule).eq_ignore_ascii_case("FFXiMain.dll") {
                        out.push((pid, exe.clone(), me.modBaseAddr as usize as u32, me.modBaseSize));
                        break;
                    }
                    mok = Module32NextW(ms, &mut me).is_ok();
                }
                let _ = CloseHandle(ms);
            }
            ok = Process32NextW(snap, &mut pe).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    Ok(out)
}

/// Opens the client read-only. With several clients running, `pid` must say which.
pub fn open(pid: Option<u32>) -> Result<Process, String> {
    let all = find_clients()?;
    let pick = match pid {
        Some(p) => all.iter().find(|c| c.0 == p).cloned().ok_or(format!("pid {p} has no FFXiMain.dll loaded"))?,
        None => match all.len() {
            0 => return Err("no process has FFXiMain.dll loaded: start the retail client (pol.exe) and enter the world first".into()),
            1 => all[0].clone(),
            _ => return Err(format!("{} clients are running ({}); pass --pid", all.len(), all.iter().map(|c| format!("{} pid {}", c.1, c.0)).collect::<Vec<_>>().join(", "))),
        },
    };
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, false, pick.0) }.map_err(|e| format!("OpenProcess({}) read-only: {e} (run the probe at the same privilege level as the game)", pick.0))?;
    Ok(Process { pid: pick.0, base: pick.2, size: pick.3, exe: pick.1, handle })
}

impl Process {
    pub fn read(&self, addr: u32, buf: &mut [u8]) -> bool {
        let mut got = 0usize;
        unsafe { ReadProcessMemory(self.handle, addr as usize as *const c_void, buf.as_mut_ptr() as *mut c_void, buf.len(), Some(&mut got)).is_ok() && got == buf.len() }
    }
    pub fn u32(&self, addr: u32) -> Option<u32> {
        let mut b = [0u8; 4];
        self.read(addr, &mut b).then(|| u32::from_le_bytes(b))
    }
    pub fn u16(&self, addr: u32) -> Option<u16> {
        let mut b = [0u8; 2];
        self.read(addr, &mut b).then(|| u16::from_le_bytes(b))
    }
    pub fn f64(&self, addr: u32) -> Option<f64> {
        let mut b = [0u8; 8];
        self.read(addr, &mut b).then(|| f64::from_le_bytes(b))
    }
    pub fn f32(&self, addr: u32) -> Option<f32> {
        self.u32(addr).map(f32::from_bits)
    }

    /// Copies the module image. Unreadable pages become zeros; the count is returned.
    pub fn read_image(&self) -> (Vec<u8>, usize) {
        let mut img = vec![0u8; self.size as usize];
        let mut bad = 0;
        for off in (0..img.len()).step_by(0x10000) {
            let end = (off + 0x10000).min(img.len());
            if !self.read(self.base + off as u32, &mut img[off..end]) {
                for p in (off..end).step_by(0x1000) {
                    let pe = (p + 0x1000).min(end);
                    if !self.read(self.base + p as u32, &mut img[p..pe]) {
                        bad += 1;
                    }
                }
            }
        }
        (img, bad)
    }
}

// ---- Raw input (wheel and relative mouse counts) --------------------------------------------

static WHEEL_120: AtomicI32 = AtomicI32::new(0);
static MOUSE_DX: AtomicI32 = AtomicI32::new(0);
static MOUSE_DY: AtomicI32 = AtomicI32::new(0);
static STOP: AtomicBool = AtomicBool::new(false);

pub fn stop_requested() -> bool {
    STOP.load(Ordering::Relaxed)
}

unsafe extern "system" fn ctrl_handler(_: u32) -> windows::core::BOOL {
    STOP.store(true, Ordering::Relaxed);
    true.into()
}

pub fn install_ctrl_c() {
    unsafe {
        let _ = windows::Win32::System::Console::SetConsoleCtrlHandler(Some(ctrl_handler), true);
    }
}

unsafe extern "system" fn raw_wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if m == WM_INPUT {
            let mut buf = [0u8; 128];
            let mut size = buf.len() as u32;
            let n = GetRawInputData(HRAWINPUT(lp.0 as *mut c_void), RID_INPUT, Some(buf.as_mut_ptr() as *mut c_void), &mut size, std::mem::size_of::<RAWINPUTHEADER>() as u32);
            // Header (24 bytes on x64): dwType, dwSize, hDevice, wParam. Then RAWMOUSE: usFlags u16
            // at 24, usButtonFlags u16 at 28, usButtonData u16 at 30, lLastX i32 at 36, lLastY i32 at 40.
            if n != u32::MAX && n >= 44 && u32::from_le_bytes(buf[0..4].try_into().unwrap()) == 0 {
                let flags = u16::from_le_bytes(buf[24..26].try_into().unwrap());
                let bflags = u16::from_le_bytes(buf[28..30].try_into().unwrap());
                let bdata = i16::from_le_bytes(buf[30..32].try_into().unwrap());
                if bflags & 0x0400 != 0 {
                    WHEEL_120.fetch_add(bdata as i32, Ordering::Relaxed);
                }
                if flags & 1 == 0 {
                    MOUSE_DX.fetch_add(i32::from_le_bytes(buf[36..40].try_into().unwrap()), Ordering::Relaxed);
                    MOUSE_DY.fetch_add(i32::from_le_bytes(buf[40..44].try_into().unwrap()), Ordering::Relaxed);
                }
            }
            return LRESULT(0);
        }
        DefWindowProcW(h, m, wp, lp)
    }
}

/// Starts the raw-input listener thread. Returns an error text if registration failed; the probe
/// then logs zeros for wheel and mouse counts and says so.
pub fn start_raw_input() -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
    std::thread::spawn(move || unsafe {
        let setup = || -> Result<HWND, String> {
            let hinst = GetModuleHandleW(None).map_err(|e| e.to_string())?;
            let class = w!("SignetParityRawInput");
            let wc = WNDCLASSEXW { cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32, lpfnWndProc: Some(raw_wndproc), hInstance: hinst.into(), lpszClassName: class, ..Default::default() };
            if RegisterClassExW(&wc) == 0 {
                return Err("RegisterClassExW failed".into());
            }
            let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), class, class, WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(hinst.into()), None).map_err(|e| e.to_string())?;
            let rid = [RAWINPUTDEVICE { usUsagePage: 1, usUsage: 2, dwFlags: RIDEV_INPUTSINK, hwndTarget: hwnd }];
            RegisterRawInputDevices(&rid, std::mem::size_of::<RAWINPUTDEVICE>() as u32).map_err(|e| e.to_string())?;
            Ok(hwnd)
        };
        match setup() {
            Err(e) => {
                let _ = tx.send(Err(e));
            }
            Ok(_) => {
                let _ = tx.send(Ok(()));
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
    });
    rx.recv().map_err(|e| e.to_string())?
}

/// Takes (and clears) the wheel notches and mouse counts since the last call.
pub fn take_mouse() -> (i32, i32, i32) {
    // Only the sampler thread calls this, so the remainder needs no stronger ordering.
    static REM: AtomicI32 = AtomicI32::new(0);
    let total = REM.load(Ordering::Relaxed) + WHEEL_120.swap(0, Ordering::Relaxed);
    REM.store(total % 120, Ordering::Relaxed);
    (total / 120, MOUSE_DX.swap(0, Ordering::Relaxed), MOUSE_DY.swap(0, Ordering::Relaxed))
}

fn down(vk: u16) -> bool {
    unsafe { (GetAsyncKeyState(vk as i32) as u16) & 0x8000 != 0 }
}

pub fn read_keys() -> Keys {
    let mut num = 0u32;
    for n in 0..10u16 {
        if down(0x60 + n) {
            num |= 1 << n;
        }
    }
    for (vk, bit) in [(0x6bu16, NUM_ADD), (0x6d, NUM_SUB), (0x6a, NUM_MUL), (0x6f, NUM_DIV)] {
        if down(vk) {
            num |= bit;
        }
    }
    let mut nav = 0u32;
    for (vk, bit) in [(0x21u16, NAV_PGUP), (0x22, NAV_PGDN), (0x24, NAV_HOME), (0x23, NAV_END), (0x2d, NAV_INS), (0x2e, NAV_DEL)] {
        if down(vk) {
            nav |= bit;
        }
    }
    Keys {
        w: down(b'W' as u16),
        a: down(b'A' as u16),
        s: down(b'S' as u16),
        d: down(b'D' as u16),
        up: down(0x26),
        down: down(0x28),
        left: down(0x25),
        right: down(0x27),
        lmb: down(0x01),
        rmb: down(0x02),
        mmb: down(0x04),
        num,
        nav,
    }
}

// ---- High-resolution clock -------------------------------------------------------------------

pub struct Clock {
    freq: i64,
}

impl Clock {
    pub fn new() -> Clock {
        let mut f = 0i64;
        unsafe {
            let _ = QueryPerformanceFrequency(&mut f);
            windows::Win32::Media::timeBeginPeriod(1);
        }
        Clock { freq: f.max(1) }
    }
    pub fn now(&self) -> i64 {
        let mut c = 0i64;
        unsafe {
            let _ = QueryPerformanceCounter(&mut c);
        }
        c
    }
    pub fn secs(&self, ticks: i64) -> f64 {
        ticks as f64 / self.freq as f64
    }
    pub fn ticks(&self, secs: f64) -> i64 {
        (secs * self.freq as f64) as i64
    }
    /// Sleeps most of the wait, then spins the last 2 ms.
    pub fn wait_until(&self, target: i64) {
        loop {
            let left = target - self.now();
            if left <= 0 {
                return;
            }
            let ms = self.secs(left) * 1000.0;
            if ms > 3.0 {
                std::thread::sleep(std::time::Duration::from_micros(((ms - 2.0) * 1000.0) as u64));
            } else {
                std::hint::spin_loop();
            }
        }
    }
}

impl Drop for Clock {
    fn drop(&mut self) {
        unsafe {
            windows::Win32::Media::timeEndPeriod(1);
        }
    }
}

// ---- One sample ----------------------------------------------------------------------------

pub struct RawSample {
    pub row: Row,
    /// Name bytes of the player entity (never written to disk; used for the self-check only).
    pub name_len: Option<usize>,
    pub server_id: Option<u32>,
    pub player_index: i32,
}

fn le_f32(b: &[u8], off: u32) -> f64 {
    let o = off as usize;
    b.get(o..o + 4).map_or(f64::NAN, |s| f32::from_le_bytes(s.try_into().unwrap()) as f64)
}

/// Reads one sample. Failure to read the entity or camera is flagged in `quality`, never hidden.
pub fn sample(p: &Process, l: &Layout, t: f64) -> RawSample {
    let mut row = Row { t, ..Default::default() };
    let mut q = 0u32;
    let (mut name_len, mut server_id, mut index) = (None, None, 0i32);

    // Entity.
    let idx = p.u16(p.base + l.ent_selector_rva).map(|v| v as i16 as i32);
    let ent = idx.and_then(|i| {
        index = i;
        (0..0x1000).contains(&i).then_some(i)
    })
    .and_then(|i| p.u32(p.base + l.ent_table_rva + 4 * i as u32))
    .filter(|&e| e != 0);
    match ent {
        Some(e) => {
            let mut b = [0u8; 0x200];
            if p.read(e, &mut b) {
                row.pos = [le_f32(&b, l.ent_pos_off), le_f32(&b, l.ent_pos_off + 4), le_f32(&b, l.ent_pos_off + 8)];
                let h = le_f32(&b, l.ent_yaw_off);
                row.heading_deg = if h.is_finite() && h.abs() < 100.0 { h.to_degrees().rem_euclid(360.0) } else { f64::NAN };
                row.speed_eff = le_f32(&b, l.ent_speed_eff_off);
                row.speed_srv = le_f32(&b, l.ent_speed_server_off);
                server_id = l.ent_id_off.and_then(|o| b.get(o as usize..o as usize + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap())));
                name_len = l.ent_name_off.map(|o| b[o as usize..o as usize + 18].iter().position(|&c| c == 0).unwrap_or(18));
                let sane = row.pos.iter().all(|v| v.is_finite() && v.abs() < 1e5) && row.heading_deg.is_finite() && (-1.0..60.0).contains(&row.speed_eff) && (-1.0..60.0).contains(&row.speed_srv);
                if !sane {
                    q |= Q_INSANE;
                }
            } else {
                q |= Q_ENTITY;
            }
        }
        None => q |= Q_ENTITY,
    }

    // Camera.
    let cam = p.u32(p.base + l.cam_global_rva).filter(|&g| g != 0).and_then(|g| p.u32(g + l.cam_ptr_off)).filter(|&c| c != 0);
    match cam {
        Some(c) => {
            let mut b = [0u8; 0x80];
            if p.read(c, &mut b) {
                row.eye = [le_f32(&b, l.cam_eye_off), le_f32(&b, l.cam_eye_off + 4), le_f32(&b, l.cam_eye_off + 8)];
                row.look = [le_f32(&b, l.cam_look_off), le_f32(&b, l.cam_look_off + 4), le_f32(&b, l.cam_look_off + 8)];
                row.derive();
                let finite = row.eye.iter().chain(&row.look).all(|v| v.is_finite() && v.abs() < 1e5);
                if !finite || !(0.05..300.0).contains(&row.dist) {
                    q |= Q_INSANE;
                }
            } else {
                q |= Q_CAMERA;
            }
        }
        None => q |= Q_CAMERA,
    }

    // Projection: 2*atan(num/den); num is a constant double, den a constant or a game-state field.
    let den = match l.fov_den {
        crate::layout::FovDen::Const(r) => p.f64(p.base + r),
        crate::layout::FovDen::Field { global_rva, off } => p.u32(p.base + global_rva).filter(|&g| g != 0).and_then(|g| p.f32(g.wrapping_add(off))).map(|v| v as f64),
    };
    if let (Some(n), Some(d)) = (p.f64(p.base + l.fov_num_rva), den) {
        if d != 0.0 && n.is_finite() && d.is_finite() {
            row.fov_deg = crate::layout::fov_deg(n, d);
        }
    }

    row.keys = read_keys();
    let (w, dx, dy) = take_mouse();
    row.wheel = w;
    row.mouse_dx = dx;
    row.mouse_dy = dy;
    row.quality = q;
    RawSample { row, name_len, server_id, player_index: index }
}
