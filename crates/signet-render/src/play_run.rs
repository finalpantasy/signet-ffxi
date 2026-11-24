//! Drivers for play mode: the interactive window loop and the scripted headless run.
//!
//! Script file, one command per line (`#` starts a comment):
//!   hold KEYS N        hold keys (letters from wasd, e.g. `w` or `wd`; `-` for none) for N ticks (60 per second)
//!   cam YAW PITCH [D]  set the follow camera angles (degrees) and optionally distance
//!   shot NAME.png      render the current state to <shot dir>/NAME.png
//!   log                print one state line now
//! While holding, a state line is printed every 30 ticks.
use std::path::Path;
use std::time::Instant;

use signet_dat::collision::Collision;
use windows::{
    Win32::{
        Foundation::{HWND, POINT, RECT},
        Graphics::{
            Direct3D12::*,
            Dxgi::{Common::*, *},
            Gdi::ClientToScreen,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    },
    core::{Interface, w},
};

use crate::actor::ActorModel;
use crate::backend::{Backend, FrameParams, Res};
use crate::d3d12::{self, Gpu};
use crate::play::{self, FixedStep, Input, Sim};
use crate::{Camera, LIGHT, key};


fn state_line(s: &Sim, col: &Collision) -> String {
    let p = s.player.pos;
    let top = col.ground_height(p[0], p[2]).map_or("none".to_string(), |g| format!("{g:.2}"));
    let below = col.ground_height_below(p[0], p[1] - play::PROBE_UP, p[2]).map_or("none".to_string(), |g| format!("{g:.2}"));
    format!(
        "t={:6.2}s tick {:5} pos ({:8.2},{:7.2},{:8.2}) ground_below {below} top {top} yaw {:5.1} run_w {:.2} {}",
        s.ticks as f32 * play::DT,
        s.ticks,
        p[0],
        p[1],
        p[2],
        s.player.yaw,
        s.blend.run,
        if s.moving { "moving" } else { "still" }
    )
}

/// Picks a start: the given xz, else the nearest spot to the zone center that has ground.
fn start_pos(col: &Collision, want: Option<([f32; 2], Option<f32>)>, center: [f32; 2]) -> Res<[f32; 3]> {
    if let Some(([x, z], y)) = want {
        let y = y.or_else(|| col.ground_height(x, z)).ok_or("no ground at the --actor position")?;
        return Ok([x, y, z]);
    }
    for r in (0..=400).step_by(8) {
        for k in 0..16 {
            let a = k as f32 * std::f32::consts::TAU / 16.0;
            let (x, z) = (center[0] + r as f32 * a.cos(), center[1] + r as f32 * a.sin());
            if let Some(y) = col.ground_height(x, z) {
                return Ok([x, y, z]);
            }
        }
    }
    Err("no ground near the zone center; pass --actor x,y,z,yaw".into())
}

/// (run speed with the modifier applied, measured stride speed of run0).
fn setup(model: &ActorModel, speed_pct: f32) -> (f32, f32) {
    let stride = match play::measure_stride(model, model.run_name()) {
        Some(s) if (1.0..=12.0).contains(&s) => {
            println!("run0 stride measured: {s:.2} yalms/s (planted-foot speed, median over the loop)");
            s
        }
        other => {
            println!("run0 stride measurement {other:?} unusable; using {} yalms/s", play::FALLBACK_STRIDE_SPEED);
            play::FALLBACK_STRIDE_SPEED
        }
    };
    let pct = play::even_speed_pct(speed_pct);
    let speed = play::run_speed_for(pct);
    println!("run speed {speed:.2} yalms/s (base {} {pct:+}%), run animation rate {:.3}", play::BASE_RUN_SPEED, play::run_anim_rate(speed, stride));
    (speed, stride)
}

pub struct PlayArgs<'a> {
    pub size: (u32, u32),
    pub start: Option<([f32; 2], Option<f32>)>,
    pub yaw: f32,
    pub script: Option<&'a Path>,
    pub shot_dir: &'a Path,
    pub center: [f32; 2],
    pub near: f32,
    pub far: f32,
    pub frames: Option<u32>,
    /// Movement-speed modifier in percent (rounded to even).
    pub speed_pct: f32,
    pub camera_mode: play::CameraMode,
    /// `--parity-log`: where to write the 60 Hz retail-parity CSV (same columns as the retail probe).
    pub parity_log: Option<&'a Path>,
}

/// The parity CSV writer, when `--parity-log` is on.
struct ParityLog(Option<signet_parity::rows::Writer>);

impl ParityLog {
    fn open(path: Option<&Path>) -> Res<ParityLog> {
        Ok(ParityLog(path.map(signet_parity::rows::Writer::create).transpose()?))
    }
    /// Logs the tick that just ran with input `i`.
    fn tick(&mut self, sim: &Sim, w: &dyn play::World, i: &Input) -> Res<()> {
        match self.0.as_mut() {
            Some(l) => l.push(&sim.parity_row(w, i)),
            None => Ok(()),
        }
    }
    fn finish(&mut self, path: Option<&Path>) -> Res<()> {
        if let Some(l) = self.0.as_mut() {
            l.flush()?;
            println!("parity log: {} rows to {}", l.rows, path.map_or(String::new(), |p| p.display().to_string()));
        }
        Ok(())
    }
}

fn keys_of(s: &str) -> Input {
    let mut i = Input::default();
    for c in s.chars() {
        match c.to_ascii_lowercase() {
            'w' => i.fwd = true,
            's' => i.back = true,
            'a' => i.left = true,
            'd' => i.right = true,
            // Camera arrows: < left, > right, ^ up, v down.
            '<' => i.cam_left = true,
            '>' => i.cam_right = true,
            '^' => i.cam_up = true,
            'v' => i.cam_down = true,
            _ => {}
        }
    }
    i
}

pub fn run_script(gpu: &mut Gpu, col: &Collision, model: &ActorModel, a: &PlayArgs, script: &Path) -> Res<()> {
    let text = std::fs::read_to_string(script).map_err(|e| format!("read script: {e}"))?;
    let start = start_pos(col, a.start, a.center)?;
    let (rs, ss) = setup(model, a.speed_pct);
    let mut sim = Sim::new(start, a.yaw, rs, ss);
    sim.set_camera_mode(a.camera_mode);
    let mut plog = ParityLog::open(a.parity_log)?;
    println!("start {:.2},{:.2},{:.2} yaw {:.1}", start[0], start[1], start[2], a.yaw);
    gpu.begin_offscreen(a.size.0, a.size.1)?;
    std::fs::create_dir_all(a.shot_dir).map_err(|e| e.to_string())?;
    let mut min_gap = f32::MAX;
    for (ln, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        let f: Vec<&str> = line.split_whitespace().collect();
        let err = |m: &str| format!("script line {}: {m}", ln + 1);
        match f.as_slice() {
            [] => {}
            ["hold", keys, n] => {
                let inp = keys_of(keys);
                let n: u32 = n.parse().map_err(|_| err("bad tick count"))?;
                for _ in 0..n {
                    sim.tick(col, &inp);
                    plog.tick(&sim, col, &inp)?;
                    if sim.ticks % 30 == 0 {
                        println!("{}", state_line(&sim, col));
                    }
                    // wall clearance check: distance to a wall in front of the character
                    let p = sim.player.pos;
                    let (s, c) = sim.player.yaw.to_radians().sin_cos();
                    if let Some(hit) = col.raycast([p[0], p[1] - play::KNEE, p[2]], [s, 0.0, c], 3.0) {
                        let t = hit.t;
                        min_gap = min_gap.min(t);
                    }
                }
            }
            // `wheel <notches>`: one tick with that many wheel notches (positive zooms in).
            ["wheel", n] => {
                let inp = Input { wheel: n.parse().map_err(|_| err("bad notch count"))?, ..Default::default() };
                sim.tick(col, &inp);
                plog.tick(&sim, col, &inp)?;
            }
            // `mouse <dx> <dy> [ticks]`: that mouse movement on each of `ticks` ticks (default 1).
            ["mouse", dx, dy, rest @ ..] => {
                let inp = Input { dx: dx.parse().map_err(|_| err("bad dx"))?, dy: dy.parse().map_err(|_| err("bad dy"))?, ..Default::default() };
                let n: u32 = rest.first().map_or(Ok(1), |t| t.parse()).map_err(|_| err("bad tick count"))?;
                for _ in 0..n {
                    sim.tick(col, &inp);
                    plog.tick(&sim, col, &inp)?;
                }
            }
            ["cam", yaw, pitch, rest @ ..] => {
                sim.cam_yaw = yaw.parse().map_err(|_| err("bad yaw"))?;
                sim.cam_pitch = pitch.parse().map_err(|_| err("bad pitch"))?;
                if let Some(d) = rest.first() {
                    sim.cam_dist = d.parse().map_err(|_| err("bad dist"))?;
                }
            }
            ["log"] => println!("{}", state_line(&sim, col)),
            ["shot", name] => {
                let cam = sim.camera(col, sim.player.pos);
                let pal = play::blended_palette(model, model.idle_name(), model.run_name(), &sim, sim.player.pos);
                gpu.set_palette(&pal);
                let fp = FrameParams {
                    view_proj: cam.view_proj_fov(a.size.0 as f32 / a.size.1 as f32, a.near, a.far, sim.camera_mode.fov_deg()),
                    light: LIGHT,
                    eye: cam.pos,
                };
                gpu.render_offscreen(&fp)?;
                let rgba = gpu.render_readback(&fp)?;
                let out = a.shot_dir.join(name);
                std::fs::write(&out, signet_dat::png::encode_rgba(a.size.0, a.size.1, &rgba)).map_err(|e| format!("write png: {e}"))?;
                println!("{}", state_line(&sim, col));
                println!("camera at ({:.2},{:.2},{:.2}), {:.2} yalms from the character", cam.pos[0], cam.pos[1], cam.pos[2], {
                    let p = sim.player.pos;
                    ((cam.pos[0] - p[0]).powi(2) + (cam.pos[1] - p[1]).powi(2) + (cam.pos[2] - p[2]).powi(2)).sqrt()
                });
                println!("wrote {}", out.display());
            }
            _ => return Err(err(&format!("unknown command `{line}`"))),
        }
    }
    plog.finish(a.parity_log)?;
    if min_gap < f32::MAX {
        println!("closest wall seen ahead of the character while holding keys: {min_gap:.2} yalms (body radius {})", play::RADIUS);
    }
    Ok(())
}

fn client_size(hwnd: HWND) -> (u32, u32) {
    let mut r = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut r);
    }
    (((r.right - r.left).max(1)) as u32, ((r.bottom - r.top).max(1)) as u32)
}

pub fn run_window(gpu: &mut Gpu, col: &Collision, model: &ActorModel, a: &PlayArgs) -> Res<()> {
    let start = start_pos(col, a.start, a.center)?;
    let (rs, ss) = setup(model, a.speed_pct);
    let mut sim = Sim::new(start, a.yaw, rs, ss);
    sim.set_camera_mode(a.camera_mode);
    let mut plog = ParityLog::open(a.parity_log)?;
    println!("start {:.2},{:.2},{:.2}. WASD move, mouse orbits, wheel zooms, Esc quits.", start[0], start[1], start[2]);
    unsafe {
        let hinst = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = w!("SignetPlay");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(crate::wndproc),
            hInstance: hinst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).map_err(|e| e.to_string())?,
            lpszClassName: class,
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            return Err("RegisterClassExW failed".into());
        }
        let mut rc = RECT { left: 0, top: 0, right: a.size.0 as i32, bottom: a.size.1 as i32 };
        let _ = AdjustWindowRect(&mut rc, WS_OVERLAPPEDWINDOW, false);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("Signet"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            rc.right - rc.left,
            rc.bottom - rc.top,
            None,
            None,
            Some(hinst.into()),
            None,
        )
        .map_err(|e| e.to_string())?;
        let factory: IDXGIFactory4 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).map_err(|e| e.to_string())?;
        let (mut w, mut h) = client_size(hwnd);
        let sc_desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: w,
            Height: h,
            Format: d3d12::COLOR_FORMAT,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            ..Default::default()
        };
        let swap1 = factory.CreateSwapChainForHwnd(&gpu.queue, hwnd, &sc_desc, None, None).map_err(|e| format!("CreateSwapChainForHwnd: {e}"))?;
        let swap: IDXGISwapChain3 = swap1.cast().map_err(|e| e.to_string())?;
        let mut buffers: Vec<ID3D12Resource> = Vec::new();
        let fetch = |gpu: &Gpu, buffers: &mut Vec<ID3D12Resource>| -> Res<()> {
            buffers.clear();
            for i in 0..2 {
                let b: ID3D12Resource = swap.GetBuffer(i).map_err(|e| e.to_string())?;
                gpu.set_rtv(1 + i, &b);
                buffers.push(b);
            }
            Ok(())
        };
        fetch(gpu, &mut buffers)?;

        let mut captured = false;
        let (mut last, mut fixed, mut frames) = (Instant::now(), FixedStep::new(), 0u32);
        let (mut fps_t, mut fps_n) = (Instant::now(), 0u32);
        'outer: loop {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    break 'outer;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let now = Instant::now();
            let dt = now.duration_since(last).as_secs_f64();
            last = now;

            // Input: only while this window is in front. The cursor is hidden and re-centered
            // each frame; the distance it moved is the mouse delta.
            let mut inp = Input::default();
            let mut mouse = (0.0f32, 0.0f32);
            if GetForegroundWindow() == hwnd {
                if key(0x1B) {
                    break;
                }
                inp = Input { fwd: key(b'W'), back: key(b'S'), left: key(b'A'), right: key(b'D'), cam_left: key(0x25), cam_up: key(0x26), cam_right: key(0x27), cam_down: key(0x28), ..Default::default() };
                let mut c = POINT { x: (w / 2) as i32, y: (h / 2) as i32 };
                let _ = ClientToScreen(hwnd, &mut c);
                let mut cur = POINT::default();
                let _ = GetCursorPos(&mut cur);
                if captured {
                    mouse = ((cur.x - c.x) as f32, (cur.y - c.y) as f32);
                } else {
                    ShowCursor(false);
                    captured = true;
                }
                let _ = SetCursorPos(c.x, c.y);
            } else if captured {
                ShowCursor(true);
                captured = false;
            }
            inp.wheel = crate::take_wheel();
            for i in 0..fixed.advance(dt) {
                // mouse and wheel apply once per frame, on the first step
                let step_in = if i == 0 { Input { dx: mouse.0, dy: mouse.1, ..inp } } else { Input { wheel: 0.0, ..inp } };
                sim.tick(col, &step_in);
                plog.tick(&sim, col, &step_in)?;
            }

            let (nw, nh) = client_size(hwnd);
            if (nw, nh) != (w, h) {
                if IsIconic(hwnd).as_bool() {
                    std::thread::sleep(std::time::Duration::from_millis(30));
                    continue;
                }
                gpu.wait_idle()?;
                buffers.clear();
                swap.ResizeBuffers(0, nw, nh, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0)).map_err(|e| format!("ResizeBuffers: {e}"))?;
                fetch(gpu, &mut buffers)?;
                (w, h) = (nw, nh);
            }
            let pos = sim.render_pos(fixed.alpha());
            let cam: Camera = sim.camera(col, pos);
            let pal = play::blended_palette(model, model.idle_name(), model.run_name(), &sim, pos);
            gpu.set_palette(&pal);
            let idx = swap.GetCurrentBackBufferIndex();
            let vp = cam.view_proj_fov(w as f32 / h as f32, a.near, a.far, sim.camera_mode.fov_deg());
            gpu.render(&buffers[idx as usize], 1 + idx, D3D12_RESOURCE_STATE_PRESENT, w, h, &vp, LIGHT, cam.pos, None)?;
            swap.Present(1, DXGI_PRESENT(0)).ok().map_err(|e| format!("Present: {e}"))?;

            frames += 1;
            fps_n += 1;
            if fps_t.elapsed().as_secs_f32() >= 1.0 {
                let ms = fps_t.elapsed().as_secs_f32() * 1000.0 / fps_n as f32;
                let p = sim.player.pos;
                let title = format!("Signet  {ms:.2} ms  pos {:.1},{:.1},{:.1}\0", p[0], p[1], p[2]);
                let wide: Vec<u16> = title.encode_utf16().collect();
                let _ = SetWindowTextW(hwnd, windows::core::PCWSTR(wide.as_ptr()));
                fps_t = Instant::now();
                fps_n = 0;
            }
            if a.frames.is_some_and(|n| frames >= n) {
                break;
            }
        }
        if captured {
            ShowCursor(true);
        }
        gpu.wait_idle()?;
        let _ = DestroyWindow(hwnd);
    }
    plog.finish(a.parity_log)?;
    Ok(())
}
