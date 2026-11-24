//! Signet zone viewer (Direct3D 12, plus a Vulkan screenshot backend).
//!
//! Usage: signet-render [zone.DAT] [--root <install>] [--size WxH]
//!                      [--screenshot out.png --camera x,y,z,yaw,pitch] [--frames N]
//!                      [--backend d3d12|vk]   (vk renders --screenshot only)
//! Zone: --zone <name|id> opens a zone by name (case-insensitive, unique prefix ok; an ambiguous
//! prefix lists the candidates) instead of a DAT path; works with --play and --screenshot.
//! Environment (D3D12): the zone's own weather keys give sky, fog and light colors. --hour H
//! (0..24, default 12), --weather NAME (default suny, then fine), --env off for the old flat look.
//! Actor: [--actor x,y,z,yaw | x,auto,z,yaw] [--anim NAME] [--anim-time SECONDS] [--no-actor]  [--look race,face,head,body,hands,legs,feet,main,sub,ranged]
//! (default: a Hume 6 yalms in front of the camera on the ground, animation idl0).
//! Play mode: --play [--actor x,auto,z,yaw] opens a third-person window (WASD move, mouse orbits,
//! wheel zooms, Esc quits); --play --script file [--shot-dir dir] runs a scripted headless proof
//! (see play_run.rs for the script format). Add --parity-log <csv> to write the 60 Hz retail-parity log.
//! Options also accept --opt=value (needed for values starting with "-"). --speed-mod PCT sets the
//! movement-speed modifier (rounded to even percent); --camera-mode retail|free picks the play camera.
//! Camera angles are degrees. Yaw 0 looks toward +z; pitch above 0 looks up
//! (FFXI's y axis points down, so up is -y). Window controls: WASD move, Q/E down/up,
//! Shift fast, hold the left mouse button and drag to look.
#![cfg(windows)]

mod actor;
mod backend;
mod d3d12;
mod env;
mod mips;
mod play;
mod play_run;
mod scene;
mod vk;

use std::path::PathBuf;
use std::time::Instant;

use backend::{Backend, FrameParams, Res};
use d3d12::Gpu;
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::{
            Direct3D12::*,
            Dxgi::{Common::*, *},
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Input::KeyboardAndMouse::GetAsyncKeyState,
            WindowsAndMessaging::*,
        },
    },
    core::{Interface, w},
};

#[derive(Clone, Copy)]
struct Camera {
    pos: [f32; 3],
    yaw: f32,   // degrees
    pitch: f32, // degrees
}

fn norm(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
    [v[0] / l, v[1] / l, v[2] / l]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

impl Camera {
    fn forward(&self) -> [f32; 3] {
        let (y, p) = (self.yaw.to_radians(), self.pitch.to_radians());
        [p.cos() * y.sin(), -p.sin(), p.cos() * y.cos()]
    }
    fn right(&self) -> [f32; 3] {
        norm(cross([0.0, -1.0, 0.0], self.forward()))
    }
    /// Row-major view-projection (left-handed, depth 0..1).
    fn view_proj(&self, aspect: f32, near: f32, far: f32) -> [f32; 16] {
        self.view_proj_fov(aspect, near, far, 60.0)
    }
    /// As `view_proj` with an explicit vertical FOV in degrees.
    fn view_proj_fov(&self, aspect: f32, near: f32, far: f32, fov_deg: f32) -> [f32; 16] {
        let f = self.forward();
        let r = self.right();
        let u = cross(f, r);
        let e = self.pos;
        let view = [
            r[0], r[1], r[2], -dot(r, e),
            u[0], u[1], u[2], -dot(u, e),
            f[0], f[1], f[2], -dot(f, e),
            0.0, 0.0, 0.0, 1.0,
        ];
        let t = (fov_deg.to_radians() * 0.5).tan();
        let proj = [
            1.0 / (aspect * t), 0.0, 0.0, 0.0,
            0.0, 1.0 / t, 0.0, 0.0,
            0.0, 0.0, far / (far - near), -far * near / (far - near),
            0.0, 0.0, 1.0, 0.0,
        ];
        let mut out = [0f32; 16];
        for i in 0..4 {
            for j in 0..4 {
                out[i * 4 + j] = (0..4).map(|k| proj[i * 4 + k] * view[k * 4 + j]).sum();
            }
        }
        out
    }
}

const LIGHT: [f32; 3] = [0.35, -0.80, 0.45]; // toward the light; -y is up

struct Args {
    dat: Option<PathBuf>,
    root: Option<PathBuf>,
    size: (u32, u32),
    shot: Option<PathBuf>,
    camera: Option<Camera>,
    frames: Option<u32>,
    /// x, y (None: ground below the camera), z, yaw degrees (0 faces +z).
    actor: Option<([f32; 2], Option<f32>, f32)>,
    anim: String,
    engaged: bool,
    anim_time: Option<f32>,
    no_actor: bool,
    vulkan: bool,
    play: bool,
    script: Option<PathBuf>,
    shot_dir: PathBuf,
    /// Movement-speed modifier in percent (rounded to even, clamped).
    speed_pct: f32,
    camera_mode: play::CameraMode,
    zone: Option<String>,
    /// Offline equipped look: race, face, head, body, hands, legs, feet, main, sub, ranged (raw ids).
    look: Option<[u16; 10]>,
    env_off: bool,
    hour: f32,
    weather: Option<String>,
    /// `--parity-log <csv>`: 60 Hz retail-parity log of the play simulation.
    parity_log: Option<PathBuf>,
}

fn parse_args() -> Res<Args> {
    let mut a = Args { dat: None, root: None, size: (1280, 720), shot: None, camera: None, frames: None, actor: None, anim: "idl0".into(), engaged: false, anim_time: None, no_actor: false, vulkan: false, play: false, script: None, shot_dir: std::env::temp_dir().join("signet-render-shots"), speed_pct: 0.0, camera_mode: play::CameraMode::Retail, zone: None, look: None, env_off: false, hour: env::DEFAULT_HOUR, weather: None, parity_log: None };
    let mut it = std::env::args().skip(1);
    while let Some(raw) = it.next() {
        // `--opt=value` form: the value may start with `-` (negative numbers).
        let (arg, mut inline) = match raw.strip_prefix("--").and_then(|_| raw.split_once('=')) {
            Some((k, v)) => (k.to_string(), Some(v.to_string())),
            None => (raw.clone(), None),
        };
        let mut val = |name: &str| inline.take().or_else(|| it.next()).ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--root" => a.root = Some(val("--root")?.into()),
            "--screenshot" => a.shot = Some(val("--screenshot")?.into()),
            "--no-actor" => a.no_actor = true,
            "--zone" => a.zone = Some(val("--zone")?),
            "--hour" => a.hour = val("--hour")?.parse().map_err(|_| "bad --hour (0..24)")?,
            "--weather" => a.weather = Some(val("--weather")?),
            "--env" => {
                a.env_off = match val("--env")?.to_ascii_lowercase().as_str() {
                    "off" => true,
                    "on" => false,
                    other => return Err(format!("unknown --env {other} (on or off)")),
                }
            }
            "--play" => a.play = true,
            "--script" => a.script = Some(val("--script")?.into()),
            "--shot-dir" => {
                a.shot_dir = val("--shot-dir")?.into();
            }
            "--parity-log" => a.parity_log = Some(val("--parity-log")?.into()),
            "--backend" => {
                a.vulkan = match val("--backend")?.to_ascii_lowercase().as_str() {
                    "d3d12" | "dx12" => false,
                    "vk" | "vulkan" => true,
                    other => return Err(format!("unknown --backend {other} (d3d12 or vk)")),
                }
            }
            "--speed-mod" => a.speed_pct = val("--speed-mod")?.trim_end_matches('%').parse().map_err(|_| "bad --speed-mod (percent, e.g. 12)")?,
            "--camera-mode" => {
                a.camera_mode = match val("--camera-mode")?.to_ascii_lowercase().as_str() {
                    "retail" => play::CameraMode::Retail,
                    "free" => play::CameraMode::Free,
                    other => return Err(format!("unknown --camera-mode {other} (retail or free)")),
                }
            }
            "--look" => {
                let v = val("--look")?;
                let f: Vec<u16> = v.split(',').map(|x| x.trim().parse().map_err(|_| "bad --look value")).collect::<Result<_, _>>()?;
                a.look = Some(f.try_into().map_err(|_| "--look race,face,head,body,hands,legs,feet,main,sub,ranged (10 numbers)")?);
            }
            "--anim" => a.anim = val("--anim")?,
            "--engaged" => a.engaged = true,
            "--anim-time" => a.anim_time = Some(val("--anim-time")?.parse().map_err(|_| "bad --anim-time")?),
            "--actor" => {
                let v = val("--actor")?;
                let f: Vec<&str> = v.split(',').map(str::trim).collect();
                if f.len() != 4 {
                    return Err("--actor x,y,z,yaw (y may be 'auto')".into());
                }
                let num = |s: &str| s.parse::<f32>().map_err(|_| format!("bad --actor number {s}"));
                let y = if f[1] == "auto" { None } else { Some(num(f[1])?) };
                a.actor = Some(([num(f[0])?, num(f[2])?], y, num(f[3])?));
            }
            "--frames" => a.frames = val("--frames")?.parse().ok(),
            "--size" => {
                let v = val("--size")?;
                let (w, h) = v.split_once(['x', 'X']).ok_or("--size WxH")?;
                a.size = (w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?);
            }
            "--camera" => {
                let n: Vec<f32> = val("--camera")?.split(',').filter_map(|s| s.trim().parse().ok()).collect();
                if n.len() != 5 {
                    return Err("--camera x,y,z,yaw,pitch".into());
                }
                a.camera = Some(Camera { pos: [n[0], n[1], n[2]], yaw: n[3], pitch: n[4] });
            }
            s if s.starts_with("--") => return Err(format!("unknown option {s}")),
            _ => a.dat = Some(raw.into()),
        }
    }
    Ok(a)
}

/// Wheel notches received since the last `take_wheel` (positive = away from the user).
static WHEEL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

fn take_wheel() -> f32 {
    WHEEL.swap(0, std::sync::atomic::Ordering::Relaxed) as f32
}

unsafe extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match m {
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let notches = ((wp.0 >> 16) as u16 as i16) as i32 / 120;
                WHEEL.fetch_add(notches, std::sync::atomic::Ordering::Relaxed);
                LRESULT(0)
            }
            _ => DefWindowProcW(h, m, wp, lp),
        }
    }
}

/// Renders the screenshot on any backend: warm-up frames, a timed loop, then one readback frame.
fn screenshot(be: &mut dyn Backend, args: &Args, out: &std::path::Path, cam: Camera, near: f32, far: f32, palette: Option<Vec<actor::Bone3x4>>) -> Res<()> {
    let (w, h) = args.size;
    be.begin_offscreen(w, h)?;
    let f = FrameParams { view_proj: cam.view_proj(w as f32 / h as f32, near, far), light: LIGHT, eye: cam.pos };
    if let Some(p) = palette {
        be.set_palette(&p);
    }
    for _ in 0..3 {
        be.render_offscreen(&f)?;
    }
    let n = 30;
    let t = Instant::now();
    for _ in 0..n {
        be.render_offscreen(&f)?;
    }
    println!("frame time {:.3} ms (avg of {n}, {w}x{h}, CPU record + submit + GPU wait)", t.elapsed().as_secs_f64() * 1000.0 / n as f64);
    let rgba = be.render_readback(&f)?;
    std::fs::write(out, signet_dat::png::encode_rgba(w, h, &rgba)).map_err(|e| format!("write png: {e}"))?;
    println!("wrote {}", out.display());
    Ok(())
}

fn key(c: u8) -> bool {
    unsafe { (GetAsyncKeyState(c as i32) as u16) & 0x8000 != 0 }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("signet-render: {e}");
        std::process::exit(1);
    }
}

fn run() -> Res<()> {
    let mut args = parse_args()?;
    let root = args.root.clone().or_else(signet_dat::find_install_root).ok_or("no FFXI install root (use --root)")?;
    let dat = match (&args.zone, &args.dat) {
        (Some(_), Some(_)) => return Err("give either --zone or a DAT path, not both".into()),
        (Some(q), None) => {
            use signet_dat::zones::{Resolved, Zones};
            let zones = Zones::from_install(&root).map_err(|e| format!("read zone table: {e}"))?;
            match zones.resolve(q) {
                Resolved::Found(z) => {
                    println!("zone {} {} -> {}", z.id, if z.name.is_empty() { "(unnamed)" } else { &z.name }, z.model.display());
                    z.model.clone()
                }
                Resolved::Ambiguous(v) => {
                    let list: Vec<String> = v.iter().map(|z| format!("  {:>4}  {}", z.id, z.name)).collect();
                    return Err(format!("--zone {q:?} matches {} zones, be more specific:
{}", v.len(), list.join("
")));
                }
                Resolved::None => return Err(format!("no zone matches {q:?} (try a zone id or a name from signet-dat-zonelist)")),
            }
        }
        (None, Some(p)) => p.clone(),
        (None, None) => root.join("ROM").join("0").join("101.DAT"),
    };
    let t0 = Instant::now();
    let mut scene = scene::load(&dat, &root)?;
    println!("{}", scene.stats);
    println!(
        "world x {:.0}..{:.0}  y {:.0}..{:.0}  z {:.0}..{:.0}  (loaded in {:.2}s)",
        scene.lo[0], scene.hi[0], scene.lo[1], scene.hi[1], scene.lo[2], scene.hi[2], t0.elapsed().as_secs_f32()
    );
    let center = [(scene.lo[0] + scene.hi[0]) / 2.0, (scene.lo[1] + scene.hi[1]) / 2.0, (scene.lo[2] + scene.hi[2]) / 2.0];
    let span = (scene.hi[0] - scene.lo[0]).max(scene.hi[2] - scene.lo[2]).max(1.0);
    let mut cam = args.camera.unwrap_or(Camera {
        pos: [center[0], scene.lo[1] - span * 0.25, scene.lo[2] - span * 0.1],
        yaw: 0.0,
        pitch: -40.0,
    });
    let far = (span * 4.0).max(2000.0);
    let near = 0.5;

    // Actor: placed on the ground below its xz (or the camera for the default spot).
    let mut model = None;
    if !args.no_actor {
        let install = signet_dat::Install::open(root.clone()).map_err(|e| format!("open install: {e}"))?;
        let mut m = match args.look {
            Some(l) => {
                let key = signet_dat::modelmap::ModelRef::Humanoid { race: l[0] as u8, face: l[1] as u8, parts: [l[2], l[3], l[4], l[5], l[6], l[7], l[8], l[9]] };
                let mut files = key.files().ok_or("no DAT mapping for --look")?;
                files.extend(key.anim_files());
                println!("look: {key:?} -> DATs {files:?}");
                actor::load_actor(&install, &files)?
            }
            None => actor::load_hume(&install)?,
        };
        m.engaged = args.engaged;
        if args.anim == "idl0" && m.armed() {
            args.anim = m.idle_name().into();
        }
        let (alo, ahi) = m.bind_extent();
        println!(
            "actor: {} bones, {} vertices, {} triangles, {} textures, {} animations; bind extent x {:.2}..{:.2} y {:.2}..{:.2} z {:.2}..{:.2}",
            m.skeleton.bones.len(), m.vertices.len(), m.indices.len() / 3, m.images.len(), m.anims.len(),
            alo[0], ahi[0], alo[1], ahi[1], alo[2], ahi[2]
        );
        let ([ax, az], ay, yaw) = args.actor.unwrap_or_else(|| {
            let (s, c) = cam.yaw.to_radians().sin_cos();
            ([cam.pos[0] + s * 6.0, cam.pos[2] + c * 6.0], None, cam.yaw + 150.0)
        });
        let y = match ay {
            Some(y) => y,
            None => actor::ground_below(&scene.vertices, &scene.indices, ax, az, cam.pos[1])
                .or_else(|| actor::ground_below(&scene.vertices, &scene.indices, ax, az, scene.lo[1]))
                .unwrap_or(cam.pos[1] + 2.0),
        };
        println!("actor placed at x {ax:.2} y {y:.2} z {az:.2} yaw {yaw:.1}");
        let slots = actor::register_textures(&mut m, &mut scene);
        model = Some((m, slots, actor::placement([ax, y, az], yaw)));
    }

    let anim_name = args.anim.clone();
    if let Some((m, _, _)) = &model {
        if m.animation(&anim_name).is_none() {
            return Err(format!("no animation {anim_name}"));
        }
    }
    let pose = |seconds: f32| model.as_ref().map(|(m, _, place)| m.pose_palette(m.animation(&anim_name), seconds, place));

    let t1 = Instant::now();
    if args.vulkan {
        let out = args.shot.as_ref().ok_or("--backend vk renders --screenshot only (the window path is D3D12)")?;
        let mut vk = vk::VkGpu::new(&scene)?;
        if let Some((m, slots, _)) = &model {
            vk.set_actor(m, slots)?;
        }
        println!("adapter: {} (Vulkan{})  GPU upload {:.2}s", vk.adapter_name(), if vk.validation { ", validation on" } else { "" }, t1.elapsed().as_secs_f32());
        screenshot(&mut vk, &args, out, cam, near, far, pose(args.anim_time.unwrap_or(0.0)))?;
        if vk.validation {
            println!(
                "validation: {} errors, {} warnings",
                vk::VALIDATION_ERRORS.load(std::sync::atomic::Ordering::Relaxed),
                vk::VALIDATION_WARNINGS.load(std::sync::atomic::Ordering::Relaxed)
            );
        }
        return Ok(());
    }
    let mut gpu = Gpu::new(&scene)?;
    if !args.env_off {
        match env::pick(scene.environment.as_ref(), args.weather.as_deref(), args.hour)? {
            Some((weather, e)) => {
                println!(
                    "environment: weather {weather} at {:.1}h  fog {:.0}..{:.0}  fog rgb {:.2},{:.2},{:.2}  ambient {:.2},{:.2},{:.2}  diffuse {:.2},{:.2},{:.2}  horizon {:.2},{:.2},{:.2}  indoor {} gain {:.2}",
                    args.hour, e.fog_start, e.fog_end, e.fog_color[0], e.fog_color[1], e.fog_color[2], e.ambient[0], e.ambient[1], e.ambient[2], e.diffuse[0], e.diffuse[1], e.diffuse[2], e.sky[0][0], e.sky[0][1], e.sky[0][2], e.indoor, e.gain
                );
                gpu.set_environment(Some(e));
            }
            None => println!("environment: none in this zone DAT; using the flat fallback (blue sky, no fog, neutral shading)"),
        }
    } else {
        println!("environment: off");
    }
    if let Some((m, slots, _)) = &model {
        gpu.set_actor(m, slots)?;
    }
    let set_pose = |gpu: &mut Gpu, seconds: f32| {
        if let Some(p) = pose(seconds) {
            gpu.set_palette(&p);
        }
    };
    println!("adapter: {}  GPU upload {:.2}s", gpu.adapter_name, t1.elapsed().as_secs_f32());

    if args.play {
        // Default spawn search starts at the zone's geometry centroid for --zone (towns and
        // mountain zones are far from their bounding-box middle), else at the box middle.
        let spawn_center = if args.zone.is_some() {
            let n = scene.vertices.len() as f64;
            let (sx, sz) = scene.vertices.iter().fold((0f64, 0f64), |a, v| (a.0 + v.pos[0] as f64, a.1 + v.pos[2] as f64));
            [(sx / n) as f32, (sz / n) as f32]
        } else {
            [center[0], center[2]]
        };
        let (m, _, _) = model.as_ref().ok_or("--play needs the actor (drop --no-actor)")?;
        let col = scene.collision.as_ref().ok_or("this zone has no decodable collision")?;
        let pa = play_run::PlayArgs {
            size: args.size,
            start: args.actor.map(|(xz, y, _)| (xz, y)),
            yaw: args.actor.map_or(0.0, |(_, _, yaw)| yaw),
            script: args.script.as_deref(),
            shot_dir: &args.shot_dir,
            center: spawn_center,
            near,
            far,
            frames: args.frames,
            speed_pct: args.speed_pct,
            camera_mode: args.camera_mode,
            parity_log: args.parity_log.as_deref(),
        };
        return match pa.script {
            Some(sc) => play_run::run_script(&mut gpu, col, m, &pa, sc),
            None => play_run::run_window(&mut gpu, col, m, &pa),
        };
    }

    if let Some(out) = &args.shot {
        let p = pose(args.anim_time.unwrap_or(0.0));
        return screenshot(&mut gpu, &args, out, cam, near, far, p);
    }

    // Window.
    unsafe {
        let hinst = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = w!("SignetZoneViewer");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).map_err(|e| e.to_string())?,
            lpszClassName: class,
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            return Err("RegisterClassExW failed".into());
        }
        let mut rc = RECT { left: 0, top: 0, right: args.size.0 as i32, bottom: args.size.1 as i32 };
        let _ = AdjustWindowRect(&mut rc, WS_OVERLAPPEDWINDOW, false);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("Signet zone viewer"),
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
        let client = |hwnd: HWND| {
            let mut r = RECT::default();
            let _ = GetClientRect(hwnd, &mut r);
            (((r.right - r.left).max(1)) as u32, ((r.bottom - r.top).max(1)) as u32)
        };
        let (mut w, mut h) = client(hwnd);
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
        fetch(&gpu, &mut buffers)?;

        let (mut last_cursor, mut dragging) = (POINT::default(), false);
        let (mut last, mut frames, mut fps_t, mut fps_n) = (Instant::now(), 0u32, Instant::now(), 0u32);
        let speed = span / 8.0;
        let clock = Instant::now();
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
            let dt = now.duration_since(last).as_secs_f32().min(0.1);
            last = now;

            // Input (only while this window is in front).
            if GetForegroundWindow() == hwnd {
                let mut c = POINT::default();
                let _ = GetCursorPos(&mut c);
                let down = key(0x01);
                if down && dragging {
                    cam.yaw += (c.x - last_cursor.x) as f32 * 0.25;
                    cam.pitch = (cam.pitch - (c.y - last_cursor.y) as f32 * 0.25).clamp(-89.0, 89.0);
                }
                dragging = down;
                last_cursor = c;
                let s = speed * dt * if key(0x10) { 4.0 } else { 1.0 };
                let (f, r) = (cam.forward(), cam.right());
                let mut mv = [0f32; 3];
                let mut add = |v: [f32; 3], k: f32| {
                    for i in 0..3 {
                        mv[i] += v[i] * k;
                    }
                };
                if key(b'W') { add(f, s) }
                if key(b'S') { add(f, -s) }
                if key(b'D') { add(r, s) }
                if key(b'A') { add(r, -s) }
                if key(b'E') { add([0.0, -1.0, 0.0], s) }
                if key(b'Q') { add([0.0, 1.0, 0.0], s) }
                for i in 0..3 {
                    cam.pos[i] += mv[i];
                }
            }

            let (nw, nh) = client(hwnd);
            if (nw, nh) != (w, h) {
                if IsIconic(hwnd).as_bool() {
                    let _ = Sleep_ms(30);
                    continue;
                }
                gpu.wait_idle()?;
                buffers.clear();
                swap.ResizeBuffers(0, nw, nh, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0)).map_err(|e| format!("ResizeBuffers: {e}"))?;
                fetch(&gpu, &mut buffers)?;
                (w, h) = (nw, nh);
            }
            let idx = swap.GetCurrentBackBufferIndex();
            set_pose(&mut gpu, args.anim_time.unwrap_or_else(|| clock.elapsed().as_secs_f32()));
            let vp = cam.view_proj(w as f32 / h as f32, near, far);
            gpu.render(&buffers[idx as usize], 1 + idx, D3D12_RESOURCE_STATE_PRESENT, w, h, &vp, LIGHT, cam.pos, None)?;
            swap.Present(1, DXGI_PRESENT(0)).ok().map_err(|e| format!("Present: {e}"))?;

            frames += 1;
            fps_n += 1;
            if fps_t.elapsed().as_secs_f32() >= 1.0 {
                let ms = fps_t.elapsed().as_secs_f32() * 1000.0 / fps_n as f32;
                let title = format!("Signet zone viewer  {ms:.2} ms  pos {:.0},{:.0},{:.0}\0", cam.pos[0], cam.pos[1], cam.pos[2]);
                let wide: Vec<u16> = title.encode_utf16().collect();
                let _ = SetWindowTextW(hwnd, windows::core::PCWSTR(wide.as_ptr()));
                fps_t = Instant::now();
                fps_n = 0;
            }
            if args.frames.is_some_and(|n| frames >= n) {
                println!("rendered {frames} frames in the window");
                break;
            }
        }
        gpu.wait_idle()?;
        let _ = DestroyWindow(hwnd);
    }
    Ok(())
}

#[allow(non_snake_case)]
fn Sleep_ms(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}
