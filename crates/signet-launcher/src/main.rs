//! Signet launcher: one native window (tao + WebView2 through wry, Mica backdrop) hosting the
//! React UI from `ui/launcher`: retail FINAL FANTASY XI through PlayOnline, plus the Signet
//! zone viewer. The UI talks to this process only through WebView2
//! postMessage; every request is checked against a command whitelist and typed arguments.
//!
//! Usage: signet-launcher [--snapshot out.json] [--dev-url http://127.0.0.1:PORT/]
#![cfg_attr(not(test), windows_subsystem = "windows")]

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod render;
#[cfg(windows)]
mod retail;
mod settings;
#[cfg(windows)]
mod update;
#[cfg(windows)]
mod winutil;

#[cfg(windows)]
fn main() {
    if let Err(e) = app::main() {
        app::fatal(&e);
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("signet-launcher runs on Windows only");
}
