//! Window, WebView, asset protocol and the IPC command bridge.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Deserialize;
use serde_json::{Value, json};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::windows::WindowBuilderExtWindows;
use tao::window::{Icon, Theme as TaoTheme, WindowBuilder};
use wry::http::{Request, Response, header::CONTENT_TYPE};
use wry::{WebViewBuilder, WebViewBuilderExtWindows, WebViewExtWindows};

use crate::render::{self, Game, Spawn};
use crate::retail::Retail;
use crate::settings::{self, Patch, Settings, Theme};
use crate::update::{Config as UpdateConfig, Updater};

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/assets.rs"));
}

/// wry serves custom protocol `signet` as this origin on Windows.
const ORIGIN: &str = "http://signet.localhost";
const MAX_MESSAGE: usize = 64 * 1024;

#[derive(Clone, serde::Serialize)]
pub struct ZoneInfo {
    pub id: u16,
    pub name: String,
    #[serde(skip)]
    pub dat: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
struct ArtEntry {
    id: u16,
    spawn: Option<[f32; 3]>,
}

pub struct App {
    install: PathBuf,
    settings_path: PathBuf,
    settings: Mutex<Settings>,
    zones: OnceLock<Result<Vec<ZoneInfo>, String>>,
    zones_ready: (Mutex<bool>, std::sync::Condvar),
    art: Vec<ArtEntry>,
    retail: Retail,
    updater: Arc<Updater>,
    game: Game,
    render_exe: Option<PathBuf>,
}

pub enum UserEvent {
    Eval(String),
    Theme(Theme),
}

pub fn fatal(msg: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    use windows::core::HSTRING;
    let _ = unsafe { MessageBoxW(None, &HSTRING::from(msg), &HSTRING::from("Signet"), MB_OK | MB_ICONERROR) };
}

fn asset(path: &str) -> Option<&'static [u8]> {
    embedded::ASSETS.iter().find(|(p, _)| *p == path).map(|(_, b)| *b)
}

fn mime(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "woff2" => "font/woff2",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}

/// Maps a request path to an embedded file. No `..`, no drive letters, no backslashes.
pub fn resolve(path: &str) -> Option<String> {
    let p = path.split(['?', '#']).next().unwrap_or("").trim_start_matches('/');
    let p = if p.is_empty() { "index.html" } else { p };
    if p.split('/').any(|s| s == ".." || s == ".") || p.contains('\\') || p.contains(':') {
        return None;
    }
    Some(p.to_string())
}

const NOT_BUILT: &str = "<!doctype html><meta charset=utf-8><body style=\"font:14px 'Segoe UI';margin:40px;color:#444\">The launcher UI is not built. Run <code>npm install</code> and <code>npm run build</code> in ui/launcher, then rebuild signet-launcher.</body>";

fn serve(req: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let csp = "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'";
    let path = resolve(req.uri().path());
    let found = path.as_deref().and_then(|p| asset(p).map(|b| (p.to_string(), b)));
    let (status, ctype, body): (u16, &str, Cow<'static, [u8]>) = match found {
        Some((p, b)) => (200, mime(&p), Cow::Borrowed(b)),
        None if path.as_deref() == Some("index.html") => (200, "text/html; charset=utf-8", Cow::Borrowed(NOT_BUILT.as_bytes())),
        None => (404, "text/plain", Cow::Borrowed(b"not found".as_slice())),
    };
    Response::builder().status(status).header(CONTENT_TYPE, ctype).header("Content-Security-Policy", csp).header("Cache-Control", "no-store").body(body).unwrap()
}

fn icon() -> Option<Icon> {
    // An original mark: a gold faceted diamond on a transparent field.
    let n = 64u32;
    let mut px = vec![0u8; (n * n * 4) as usize];
    let c = (n as f32 - 1.0) / 2.0;
    for y in 0..n {
        for x in 0..n {
            let (dx, dy) = ((x as f32 - c) / c, (y as f32 - c) / c);
            let d = dx.abs() * 0.82 + dy.abs();
            if d > 0.92 {
                continue;
            }
            let edge = ((0.92 - d) * 40.0).clamp(0.0, 1.0);
            let facet = if dx < 0.0 && dy < 0.0 { 1.0 } else if dx >= 0.0 && dy < 0.0 { 0.86 } else if dx < 0.0 { 0.74 } else { 0.62 };
            let inner = d < 0.42;
            let (r, g, b) = if inner { (250.0, 236.0, 196.0) } else { (214.0 * facet, 172.0 * facet, 92.0 * facet) };
            let i = ((y * n + x) * 4) as usize;
            px[i..i + 4].copy_from_slice(&[r as u8, g as u8, b as u8, (255.0 * edge) as u8]);
        }
    }
    Icon::from_rgba(px, n, n).ok()
}

impl App {
    fn new() -> Arc<Self> {
        let install = match std::env::var_os("SIGNET_INSTALL") {
            // Test seam: point the launcher at a made-up install instead of the real one.
            Some(p) if !p.is_empty() => PathBuf::from(p),
            _ => {
                let d = PathBuf::from(settings::DEFAULT_INSTALL);
                if d.join("FFXiMain.dll").exists() { d } else { signet_dat::find_install_root().unwrap_or(d) }
            }
        };
        let settings_path = settings::settings_path();
        let s = settings::load(&settings_path);
        let art = asset("zones/index.json").and_then(|b| serde_json::from_slice::<Vec<ArtEntry>>(b).ok()).unwrap_or_default();
        let app = Arc::new(App {
            retail: Retail::new(install.clone()),
            updater: Updater::new(UpdateConfig::from_env(install.clone())),
            install,
            settings_path,
            settings: Mutex::new(s),
            zones: OnceLock::new(),
            zones_ready: (Mutex::new(false), std::sync::Condvar::new()),
            art,
            game: Game::default(),
            render_exe: render::find_exe(),
        });
        let a = app.clone();
        std::thread::spawn(move || {
            let r = signet_dat::zones::Zones::from_install(a.install.clone())
                .map(|z| z.iter().filter(|z| !z.name.is_empty()).map(|z| ZoneInfo { id: z.id, name: z.name.clone(), dat: z.model.clone() }).collect())
                .map_err(|e| format!("read zones from {}: {e}", a.install.display()));
            let _ = a.zones.set(r);
            *a.zones_ready.0.lock().unwrap() = true;
            a.zones_ready.1.notify_all();
        });
        app
    }

    fn zones(&self) -> Result<&Vec<ZoneInfo>, String> {
        let mut ready = self.zones_ready.0.lock().unwrap();
        while !*ready {
            ready = self.zones_ready.1.wait(ready).unwrap();
        }
        drop(ready);
        self.zones.get().unwrap().as_ref().map_err(Clone::clone)
    }

    fn zone(&self, id: u16) -> Result<ZoneInfo, String> {
        self.zones()?.iter().find(|z| z.id == id).cloned().ok_or_else(|| format!("zone {id} is not in this install"))
    }

    fn game_json(&self) -> Value {
        let running = self.game.running();
        json!({
            "running": running.is_some(),
            "pid": running.map(|r| r.0),
            "zone": running.map(|r| r.1),
            "lastExit": self.game.last_exit().map(|(c, l)| json!({ "code": c, "line": l })),
            "exe": self.render_exe,
            "zoneFlag": self.render_exe.as_deref().is_some_and(render::supports_zone_flag),
        })
    }

    fn init(&self) -> Value {
        json!({
            "settings": *self.settings.lock().unwrap(),
            "paths": {
                "install": self.install,
                "settings": self.settings_path,
                "viewer": self.retail.viewer,
                "renderLog": render::log_path(),
            },
            "resolutions": settings::RESOLUTIONS,
            "speed": [settings::SPEED_MIN, settings::SPEED_MAX],
            "game": self.game_json(),
            "version": env!("CARGO_PKG_VERSION"),
        })
    }

    fn set_settings(&self, args: Value, proxy: &EventLoopProxy<UserEvent>) -> Result<Value, String> {
        let patch: Patch = serde_json::from_value(args).map_err(|e| format!("bad settings: {e}"))?;
        let needs_zone = patch.zone_id.is_some() || patch.favorite.is_some();
        let ids: Vec<u16> = if needs_zone { self.zones()?.iter().map(|z| z.id).collect() } else { Vec::new() };
        let theme = patch.theme;
        let mut s = self.settings.lock().unwrap();
        s.apply(patch, &|z| ids.contains(&z))?;
        settings::save(&self.settings_path, &s)?;
        if let Some(t) = theme {
            let _ = proxy.send_event(UserEvent::Theme(t));
        }
        Ok(serde_json::to_value(&*s).unwrap())
    }

    fn status(&self) -> Value {
        json!({ "game": self.game_json(), "retail": { "pid": self.retail.running(), "blocked": self.updater.blocked() } })
    }

    fn explore(&self, args: Value) -> Result<Value, String> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct A {
            zone_id: Option<u16>,
        }
        let a: A = serde_json::from_value(args).map_err(|e| format!("bad explore request: {e}"))?;
        let exe = self.render_exe.clone().ok_or("signet-render.exe was not found next to the launcher or in target\\release.")?;
        let mut s = self.settings.lock().unwrap();
        let zone = self.zone(a.zone_id.unwrap_or(s.zone_id))?;
        let spawn = self.art.iter().find(|e| e.id == zone.id).and_then(|e| e.spawn).map(|[x, z, yaw]| Spawn { x, z, yaw });
        let argv = render::args(&s, zone.id, &zone.dat, &self.install, spawn, render::supports_zone_flag(&exe));
        let pid = self.game.launch(&exe, &argv, zone.id)?;
        s.zone_id = zone.id;
        s.push_recent(zone.id);
        let _ = settings::save(&self.settings_path, &s);
        Ok(json!({ "pid": pid, "zone": zone.id, "args": argv }))
    }

    fn dispatch(&self, cmd: &str, args: Value, proxy: &EventLoopProxy<UserEvent>) -> Result<Value, String> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Open {
            target: String,
        }
        match cmd {
            "init" => Ok(self.init()),
            "zones" => Ok(json!(self.zones()?)),
            "settings.set" => self.set_settings(args, proxy),
            "explore" => self.explore(args),
            "explore.stop" => self.game.stop(std::time::Duration::from_secs(15)).map(|_| self.game_json()),
            "status" => Ok(self.status()),
            "retail.info" => Ok(self.retail.info()),
            "retail.play" => self.retail.play().map(|pid| json!({ "pid": pid })),
            "update.state" => {
                let mut v = self.updater.state();
                v["blocked"] = json!(self.updater.blocked());
                Ok(v)
            }
            "update.check" => Ok(self.updater.check()),
            "update.plan" => self.updater.plan(),
            "update.apply" => self.updater.apply(),
            "update.restore" => self.updater.restore(),
            "update.cancel" => {
                self.updater.cancel();
                Ok(self.updater.state())
            }
            "update.dismiss" => Ok(self.updater.dismiss()),
            "open" => {
                let o: Open = serde_json::from_value(args).map_err(|e| format!("bad open request: {e}"))?;
                let dir = match o.target.as_str() {
                    "install" => self.install.clone(),
                    "settings" => settings::settings_dir(),
                    "viewer" => self.retail.viewer.clone(),
                    "backups" => self.updater.backups_dir().to_path_buf(),
                    _ => return Err("unknown folder".into()),
                };
                crate::winutil::open_folder(&dir).map(|_| Value::Null)
            }
            _ => Err(format!("unknown command {cmd}")),
        }
    }

    /// Everything a dev build of the UI needs to render real data without the native host.
    fn snapshot(&self) -> Value {
        json!({
            "init": self.init(),
            "zones": self.zones().cloned().unwrap_or_default(),
            "status": self.status(),
            "retail": self.retail.info(),
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IpcMessage {
    id: u32,
    cmd: String,
    #[serde(default)]
    args: Value,
}

/// Parses one postMessage body. Oversized or malformed messages are dropped.
fn parse_ipc(body: &str) -> Option<IpcMessage> {
    if body.len() > MAX_MESSAGE {
        return None;
    }
    let m: IpcMessage = serde_json::from_str(body).ok()?;
    if m.cmd.len() > 32 || !m.cmd.bytes().all(|b| b.is_ascii_lowercase() || b == b'.') {
        return None;
    }
    Some(m)
}

fn allowed_origin(url: &str, dev: Option<&str>) -> bool {
    let ok = |base: &str| url == base || url.starts_with(&format!("{}/", base.trim_end_matches('/')));
    ok(ORIGIN) || dev.is_some_and(ok)
}

fn reply_js(id: u32, r: Result<Value, String>) -> String {
    let body = match r {
        Ok(v) => json!({ "id": id, "ok": true, "result": v }),
        Err(e) => json!({ "id": id, "ok": false, "error": e }),
    };
    format!("window.__signetReply && window.__signetReply({body})")
}

fn apply_theme(window: &tao::window::Window, webview: &wry::WebView, t: Theme) {
    let (tt, wt) = match t {
        Theme::System => (None, wry::Theme::Auto),
        Theme::Light => (Some(TaoTheme::Light), wry::Theme::Light),
        Theme::Dark => (Some(TaoTheme::Dark), wry::Theme::Dark),
    };
    window.set_theme(tt);
    let _ = webview.set_theme(wt);
    let dark = match t {
        Theme::System => window.theme() == TaoTheme::Dark,
        Theme::Dark => true,
        Theme::Light => false,
    };
    let _ = window_vibrancy::apply_mica(window, Some(dark));
    // Mica shows only behind frame areas: extend the frame over the whole client so it shows
    // through the transparent WebView2 (the window has no GDI redirection bitmap to cover it).
    use tao::platform::windows::WindowExtWindows;
    let m = windows::Win32::UI::Controls::MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
    let _ = unsafe { windows::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea(windows::Win32::Foundation::HWND(window.hwnd() as _), &m) };
}

pub fn main() -> Result<(), String> {
    // WebView2 paints this before the first frame; transparent lets the Mica backdrop show.
    // SAFETY: set first thing on the main thread, before any other thread exists.
    unsafe { std::env::set_var("WEBVIEW2_DEFAULT_BACKGROUND_COLOR", "00000000") };
    let mut args = std::env::args().skip(1);
    let mut dev_url: Option<String> = None;
    let mut snapshot: Option<PathBuf> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--dev-url" => {
                let u = args.next().ok_or("--dev-url needs a URL")?;
                if !(u.starts_with("http://127.0.0.1:") || u.starts_with("http://localhost:")) {
                    return Err("--dev-url must be a loopback http URL".into());
                }
                dev_url = Some(u);
            }
            "--snapshot" => snapshot = Some(args.next().ok_or("--snapshot needs a file")?.into()),
            _ => return Err(format!("unknown option {a}")),
        }
    }
    let app = App::new();
    if let Some(out) = snapshot {
        let body = serde_json::to_vec_pretty(&app.snapshot()).map_err(|e| e.to_string())?;
        return std::fs::write(&out, body).map_err(|e| format!("write {}: {e}", out.display()));
    }
    run_window(app, dev_url)
}

fn run_window(app: Arc<App>, dev_url: Option<String>) -> Result<(), String> {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let theme = app.settings.lock().unwrap().theme;
    let window = WindowBuilder::new()
        .with_title("Signet")
        .with_inner_size(tao::dpi::LogicalSize::new(1280.0, 800.0))
        .with_min_inner_size(tao::dpi::LogicalSize::new(1040.0, 680.0))
        .with_transparent(true)
        // A GDI redirection surface paints white under WebView2; without one, Mica shows through.
        .with_no_redirection_bitmap(true)
        .with_window_icon(icon())
        .with_theme(match theme {
            Theme::Dark => Some(TaoTheme::Dark),
            Theme::Light => Some(TaoTheme::Light),
            Theme::System => None,
        })
        .build(&event_loop)
        .map_err(|e| format!("create window: {e}"))?;

    // Mica needs Windows 11; without it the page paints its own opaque backdrop.
    let dark = match theme {
        Theme::System => window.theme() == TaoTheme::Dark,
        Theme::Dark => true,
        Theme::Light => false,
    };
    let mica = window_vibrancy::apply_mica(&window, Some(dark)).is_ok();
    let url = dev_url.clone().unwrap_or_else(|| format!("{ORIGIN}/index.html"));
    let dev_nav = dev_url.clone();
    let dev_ipc = dev_url.clone();
    let ipc_app = app.clone();
    let ipc_proxy = proxy.clone();
    let update_proxy = proxy.clone();
    app.updater.set_emitter(move |state| {
        let ev = json!({ "type": "update", "state": state });
        let _ = update_proxy.send_event(UserEvent::Eval(format!("window.__signetEvent && window.__signetEvent({ev})")));
    });
    let webview = WebViewBuilder::new()
        .with_url(url)
        .with_transparent(true)
        .with_background_color((0, 0, 0, 0))
        .with_devtools(cfg!(debug_assertions))
        .with_default_context_menus(cfg!(debug_assertions))
        .with_browser_accelerator_keys(false)
        .with_initialization_script(&format!("window.__SIGNET_NATIVE__ = true; window.__SIGNET_MICA__ = {mica};"))
        .with_custom_protocol("signet".into(), |_id, req| serve(req))
        .with_navigation_handler(move |u| allowed_origin(&u, dev_nav.as_deref()))
        .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
        .with_ipc_handler(move |req: Request<String>| {
            let origin = req.uri().to_string();
            if !allowed_origin(&origin, dev_ipc.as_deref()) {
                return;
            }
            let Some(m) = parse_ipc(req.body()) else { return };
            let app = ipc_app.clone();
            let proxy = ipc_proxy.clone();
            std::thread::spawn(move || {
                let r = app.dispatch(&m.cmd, m.args, &proxy);
                let _ = proxy.send_event(UserEvent::Eval(reply_js(m.id, r)));
            });
        })
        .build(&window)
        .map_err(|e| format!("create WebView2 (is the WebView2 runtime installed?): {e}"))?;
    apply_theme(&window, &webview, theme);

    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(UserEvent::Eval(js)) => {
                let _ = webview.evaluate_script(&js);
            }
            Event::UserEvent(UserEvent::Theme(t)) => apply_theme(&window, &webview, t),
            Event::WindowEvent { event: WindowEvent::ThemeChanged(_), .. } => {
                let t = app.settings.lock().unwrap().theme;
                if t == Theme::System {
                    apply_theme(&window, &webview, t);
                }
            }
            // The zone viewer and PlayOnline are separate programs; closing the launcher leaves them running.
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => *flow = ControlFlow::Exit,
            _ => {}
        }
    })
}

#[allow(dead_code)]
fn _assert_paths(_: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_paths_cannot_escape() {
        assert_eq!(resolve("/").as_deref(), Some("index.html"));
        assert_eq!(resolve("/assets/app.js?v=1").as_deref(), Some("assets/app.js"));
        assert_eq!(resolve("/../private/database.json"), None);
        assert_eq!(resolve("/zones/./x.jpg"), None);
        assert_eq!(resolve("/C:/Windows/win.ini"), None);
        assert_eq!(resolve("/a\\b"), None);
    }

    #[test]
    fn ipc_messages_are_strict() {
        assert!(parse_ipc(r#"{"id":1,"cmd":"status"}"#).is_some());
        assert!(parse_ipc(r#"{"id":1,"cmd":"status","args":{},"extra":1}"#).is_none());
        assert!(parse_ipc(r#"{"id":1,"cmd":"Server Start"}"#).is_none());
        assert!(parse_ipc(r#"{"id":-1,"cmd":"init"}"#).is_none());
        assert!(parse_ipc(&format!(r#"{{"id":1,"cmd":"init","args":"{}"}}"#, "x".repeat(MAX_MESSAGE))).is_none());
    }

    #[test]
    fn only_the_app_origin_may_navigate_or_call() {
        assert!(allowed_origin("http://signet.localhost/index.html", None));
        assert!(!allowed_origin("http://signet.localhost.evil.com/", None));
        assert!(!allowed_origin("https://example.com/", None));
        assert!(allowed_origin("http://127.0.0.1:5199/", Some("http://127.0.0.1:5199/")));
        assert!(!allowed_origin("http://127.0.0.1:5200/", Some("http://127.0.0.1:5199/")));
    }

    #[test]
    fn mime_types() {
        assert_eq!(mime("index.html"), "text/html; charset=utf-8");
        assert_eq!(mime("zones/west-ronfaure.jpg"), "image/jpeg");
    }
}
