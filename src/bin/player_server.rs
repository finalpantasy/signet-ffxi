//! Same-origin loopback adapter for the player. Never serves workspace files.
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use windower_rs::{player::Player, player_host::Session, runtime_options::NativeLibrary};
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    StartOffline {
        id: String,
    },
    Runtime {
        operation: String,
        name: Option<String>,
    },
    StopRuntime {},
    Packages {},
    Shutdown {},
    Profile {
        request: windower_rs::player::Request,
    },
}
struct Server {
    player: Player,
    dist: PathBuf,
    host: Option<NativeLibrary>,
    session: Option<Session>,
    address: String,
    stopping: bool,
}
impl Server {
    fn rpc(&mut self, value: Value) -> Result<Value, String> {
        if value["schema_version"] != 1 {
            return Err("Unsupported player contract version".into());
        }
        let mut raw = value;
        raw.as_object_mut()
            .ok_or("Expected request")?
            .remove("schema_version");
        let command: Command = serde_json::from_value(raw).map_err(|_| "Invalid player command")?;
        match command {
            Command::Shutdown {} => {
                self.stopping = true;
                let runtime = self
                    .session
                    .take()
                    .map(|mut session| session.shutdown())
                    .transpose()?;
                Ok(json!({"stopped":true,"runtime":runtime}))
            }
            Command::Profile { request } => self.player.execute(request),
            Command::Packages {} => windower_rs::package_editor::execute(
                self.player.root(),
                windower_rs::package_editor::Request::Inspect,
            ),
            Command::StartOffline { id } => {
                if self.session.is_some() {
                    return Err("Stop the running offline session first".into());
                }
                let doc = self.player.current_profile(&id)?;
                let stamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_nanos();
                let parent = self.player.root().join("player-sessions");
                fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
                let host = self
                    .host
                    .as_ref()
                    .ok_or("Offline runtime host has not been configured")?;
                let session =
                    Session::start(host, id, doc, &parent.join(format!("session-{stamp}")))?;
                let view = session.view();
                self.session = Some(session);
                Ok(view)
            }
            Command::Runtime { operation, name } => {
                let running = self
                    .session
                    .as_ref()
                    .ok_or("No owned offline runtime is running")?;
                if !["status", "load", "reload", "unload"].contains(&operation.as_str()) {
                    return Err("Unsupported runtime operation".into());
                }
                if operation == "status" {
                    if name.is_some() {
                        return Err("Status does not take an addon name".into());
                    }
                } else if !running
                    .launched
                    .profile
                    .addons
                    .iter()
                    .any(|a| Some(a.name.as_str()) == name.as_deref())
                {
                    return Err("Addon is not selected in the running session".into());
                }
                let result = self
                    .session
                    .as_mut()
                    .ok_or("No owned offline runtime is running")?
                    .control(&operation, name.as_deref());
                if result.is_err() {
                    self.session = None;
                }
                result
            }
            Command::StopRuntime {} => {
                let Some(mut session) = self.session.take() else {
                    return Err("No owned offline runtime is running".into());
                };
                let reply = session.shutdown()?;
                drop(session);
                Ok(reply)
            }
        }
    }
    fn snapshot(&mut self) -> Result<Value, String> {
        if let Some(session) = self.session.as_mut()
            && !session.is_running()?
        {
            self.session = None;
        }
        let mut snapshot = self.player.snapshot()?;
        snapshot["offline_host_available"] = self.host.is_some().into();
        snapshot["session"] = self
            .session
            .as_ref()
            .map(Session::view)
            .unwrap_or(Value::Null);
        Ok(snapshot)
    }
    fn handle(&mut self, stream: &mut TcpStream) -> Result<(), String> {
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .map_err(|e| e.to_string())?;
        let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
        let mut line = String::new();
        reader
            .by_ref()
            .take(8193)
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        if line.len() > 8192 {
            return reply(stream, 413, "text/plain", b"Request too large");
        }
        let parts: Vec<_> = line.split_whitespace().collect();
        if parts.len() != 3 {
            return reply(stream, 400, "text/plain", b"Invalid request");
        }
        let method = parts[0].to_string();
        let path = parts[1].to_string();
        let mut headers = std::collections::BTreeMap::new();
        let mut total = line.len();
        loop {
            line.clear();
            reader
                .by_ref()
                .take(8193)
                .read_line(&mut line)
                .map_err(|e| e.to_string())?;
            total += line.len();
            if total > 16384 {
                return reply(stream, 413, "text/plain", b"Headers too large");
            }
            if line == "\r\n" {
                break;
            }
            let Some((key, value)) = line.trim_end().split_once(':') else {
                return reply(stream, 400, "text/plain", b"Invalid header");
            };
            if headers
                .insert(key.to_ascii_lowercase(), value.trim().to_string())
                .is_some()
            {
                return reply(stream, 400, "text/plain", b"Duplicate header");
            }
        }
        if headers.get("host") != Some(&self.address) || headers.contains_key("transfer-encoding") {
            return reply(stream, 403, "text/plain", b"Host refused");
        }
        // Consume a bounded body before a policy refusal so Windows can send
        // the response without resetting a socket with unread request bytes.
        let length = match headers.get("content-length") {
            Some(value) => value
                .parse::<usize>()
                .ok()
                .filter(|n| *n <= 65536)
                .ok_or("Invalid request size")?,
            None => 0,
        };
        let mut bytes = vec![0; length];
        reader.read_exact(&mut bytes).map_err(|e| e.to_string())?;
        if let Some(origin) = headers.get("origin")
            && origin != &format!("http://{}", self.address)
        {
            return reply(stream, 403, "text/plain", b"Origin refused");
        }
        if method == "GET" && path == "/api/v1/snapshot" {
            let snapshot = self.snapshot()?;
            return json_reply(stream, 200, &snapshot);
        }
        if method == "POST" && path == "/api/v1/rpc" {
            if headers.get("origin") != Some(&format!("http://{}", self.address))
                || headers.get("x-signet-request").map(String::as_str) != Some("1")
                || headers.get("content-type").map(String::as_str) != Some("application/json")
            {
                return reply(stream, 403, "text/plain", b"Same-origin JSON required");
            }
            let result = serde_json::from_slice(&bytes)
                .map_err(|_| "Invalid JSON".to_string())
                .and_then(|v| self.rpc(v));
            return match result {
                Ok(value) => json_reply(stream, 200, &json!({"ok":true,"value":value})),
                Err(error) => json_reply(stream, 409, &json!({"ok":false,"error":error})),
            };
        }
        if method != "GET" {
            return reply(stream, 405, "text/plain", b"Method refused");
        }
        let relative = if path == "/" || path == "/?native=1" {
            "index.html"
        } else {
            path.trim_start_matches('/')
        };
        if relative
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
            || relative.contains(['\\', '%', ':', '?', '#'])
        {
            return reply(stream, 404, "text/plain", b"Not found");
        }
        let file = self.dist.join(relative);
        let canonical = match file.canonicalize() {
            Ok(p) if p.starts_with(&self.dist) => p,
            _ => return reply(stream, 404, "text/plain", b"Not found"),
        };
        let mime = match file.extension().and_then(|s| s.to_str()) {
            Some("html") => "text/html; charset=utf-8",
            Some("js") => "text/javascript",
            Some("css") => "text/css",
            Some("svg") => "image/svg+xml",
            _ => return reply(stream, 404, "text/plain", b"Not found"),
        };
        let mut bytes = fs::read(canonical).map_err(|e| e.to_string())?;
        if relative == "index.html" {
            let html = String::from_utf8(bytes).map_err(|_| "Invalid player index")?;
            bytes = html
                .replacen(
                    "<head>",
                    "<head><meta name=\"signet-native\" content=\"1\">",
                    1,
                )
                .into_bytes();
        }
        reply(stream, 200, mime, &bytes)
    }
}
fn json_reply(stream: &mut TcpStream, status: u16, value: &Value) -> Result<(), String> {
    reply(
        stream,
        status,
        "application/json",
        &serde_json::to_vec(value).map_err(|e| e.to_string())?,
    )
}
fn reply(stream: &mut TcpStream, status: u16, mime: &str, bytes: &[u8]) -> Result<(), String> {
    write!(stream,"HTTP/1.1 {status} Response\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; frame-ancestors 'none'\r\n\r\n",bytes.len()).and_then(|_|stream.write_all(bytes)).map_err(|e|e.to_string())
}
fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 && args.len() != 5 {
        return Err("Usage: player_server WORKSPACE DIST PORT [OFFLINE_HOST SHA256]".into());
    }
    let player = Player::open(Path::new(&args[0]))?;
    let dist = Path::new(&args[1])
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !dist.join("index.html").is_file() {
        return Err("Build the player interface first".into());
    }
    let port = args[2].parse::<u16>().map_err(|_| "Invalid port")?;
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|e| e.to_string())?
        .to_string();
    let host = if args.len() == 5 {
        Some(NativeLibrary {
            path: args[3].clone().into(),
            sha256: args[4].clone(),
        })
    } else {
        None
    };
    let mut server = Server {
        player,
        dist,
        host,
        session: None,
        address: address.clone(),
        stopping: false,
    };
    println!(
        "{}",
        json!({"schema_version":1,"address":address,"process_id":std::process::id(),"mode":"native_profiles_offline_runtime"})
    );
    for stream in listener.incoming() {
        let mut stream = stream.map_err(|e| e.to_string())?;
        if server.handle(&mut stream).is_err() {
            let _ = json_reply(
                &mut stream,
                400,
                &json!({"ok":false,"error":"Request failed; refresh and inspect the owned workspace"}),
            );
        }
        if server.stopping {
            break;
        }
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
