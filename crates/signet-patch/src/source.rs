//! Where patch data comes from. Everything that touches the network is in
//! [`LiveServer`]; the rest of the crate only sees the [`Source`] traits, so
//! tests and offline runs plug in [`MemoryServer`] or a saved catalog.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::codec;
use crate::error::{PatchError, Result};
use crate::wire::{self, Ident, ServerVersion};

/// Largest decoded catalog accepted.
pub const CATALOG_LIMIT: usize = 256 << 20;

/// One open session with a patch server.
pub trait Session: Send {
    fn version(&mut self) -> Result<ServerVersion>;
    /// The decoded catalog text.
    fn catalog(&mut self) -> Result<Vec<u8>>;
    /// Read `len` bytes of a server file. `on_bytes` gets the running count.
    fn fetch(&mut self, path: &str, len: u64, on_bytes: &mut dyn FnMut(u64)) -> Result<Vec<u8>>;
}

/// Opens sessions. Shared by the download workers.
pub trait Source: Sync {
    fn open(&self) -> Result<Box<dyn Session>>;
    /// Short description for messages ("pc001.pol.com:53001").
    fn describe(&self) -> String;
}

/// Square Enix's patch server (or anything that speaks the same frames).
#[derive(Debug, Clone)]
pub struct LiveServer {
    pub host: String,
    pub port: u16,
    pub ident: Ident,
    pub timeout: Duration,
}

impl LiveServer {
    /// The FINAL FANTASY XI patch server for the Windows client.
    pub fn retail_ffxi() -> Self {
        let (host, port) = wire::server_for_title(1);
        LiveServer { host, port, ident: Ident::PC_FFXI, timeout: Duration::from_secs(60) }
    }
}

impl Source for LiveServer {
    fn open(&self) -> Result<Box<dyn Session>> {
        let addrs: Vec<_> = (self.host.as_str(), self.port)
            .to_socket_addrs()
            .map_err(|e| PatchError::failed(format!("cannot look up {}: {e}", self.describe())))?
            .collect();
        let mut last = None;
        for addr in addrs {
            match TcpStream::connect_timeout(&addr, self.timeout) {
                Ok(stream) => {
                    stream.set_read_timeout(Some(self.timeout)).ok();
                    stream.set_write_timeout(Some(self.timeout)).ok();
                    stream.set_nodelay(true).ok();
                    return Ok(Box::new(LiveSession { stream, ident: self.ident }));
                }
                Err(e) => last = Some(e),
            }
        }
        Err(PatchError::failed(format!(
            "cannot connect to {}: {}",
            self.describe(),
            last.map(|e| e.to_string()).unwrap_or_else(|| "no address".into())
        )))
    }

    fn describe(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

struct LiveSession {
    stream: TcpStream,
    ident: Ident,
}

impl LiveSession {
    /// Send one frame, read one frame back, and check it is the expected kind.
    fn call(&mut self, request: &[u8], expect: u32, max_reply: usize) -> Result<Vec<u8>> {
        self.stream
            .write_all(request)
            .map_err(|e| PatchError::failed(format!("sending to the patch server: {e}")))?;
        let mut head = [0u8; 4];
        self.stream
            .read_exact(&mut head)
            .map_err(|e| PatchError::failed(format!("reading from the patch server: {e}")))?;
        let total = u32::from_le_bytes(head) as usize;
        if !(wire::HEADER..=max_reply).contains(&total) {
            return Err(PatchError::failed(format!(
                "patch server announced a {total}-byte reply (sent {})",
                wire::hex_head(request, 24)
            )));
        }
        let mut frame = vec![0u8; total];
        frame[..4].copy_from_slice(&head);
        self.stream
            .read_exact(&mut frame[4..])
            .map_err(|e| PatchError::failed(format!("reading a {total}-byte reply: {e}")))?;
        let op = wire::decode(&frame)
            .map_err(|e| e.context(format!("reply {}", wire::hex_head(&frame, 32))))?
            .operation;
        if op == wire::op::REFUSED {
            return Err(PatchError::failed(format!(
                "patch server refused the request (sent {}; got {})",
                wire::hex_head(request, 24),
                wire::hex_head(&frame, 32)
            )));
        }
        if op != expect {
            return Err(PatchError::failed(format!(
                "patch server answered with operation {op}, expected {expect} (got {})",
                wire::hex_head(&frame, 32)
            )));
        }
        Ok(frame)
    }
}

impl Session for LiveSession {
    fn version(&mut self) -> Result<ServerVersion> {
        // An empty local version, as a fresh viewer sends.
        let frame = self.call(&wire::ask_version(&self.ident, b""), wire::op::VERSION, wire::MAX_REPLY)?;
        wire::parse_version(&frame[wire::HEADER..])
    }

    fn catalog(&mut self) -> Result<Vec<u8>> {
        let frame = self.call(&wire::hello(&self.ident), wire::op::CATALOG, wire::MAX_CATALOG_REPLY)?;
        let coded = wire::catalog_body(&frame[wire::HEADER..])?;
        codec::expand_slc(coded, CATALOG_LIMIT).map_err(|e| e.context("catalog"))
    }

    fn fetch(&mut self, path: &str, len: u64, on_bytes: &mut dyn FnMut(u64)) -> Result<Vec<u8>> {
        if len > u32::MAX as u64 {
            return Err(PatchError::failed(format!("{path}: {len} bytes is too large to request")));
        }
        let mut data = Vec::with_capacity(len as usize);
        while (data.len() as u64) < len {
            let offset = data.len() as u32;
            let want = ((len - data.len() as u64) as u32).min(wire::SLICE);
            let frame = self.call(&wire::read(&self.ident, path, offset, want), wire::op::CHUNK, wire::MAX_REPLY)?;
            let (at, chunk) = wire::parse_chunk(&frame[wire::HEADER..])?;
            if at != offset || chunk.is_empty() || chunk.len() > want as usize {
                return Err(PatchError::failed(format!(
                    "{path}: asked for {want} bytes at {offset}, got {} at {at}",
                    chunk.len()
                )));
            }
            data.extend_from_slice(chunk);
            on_bytes(data.len() as u64);
        }
        Ok(data)
    }
}

/// An in-memory server for tests and for offline planning against a saved
/// catalog. Fetches of unknown paths fail.
#[derive(Debug, Clone, Default)]
pub struct MemoryServer {
    pub version: String,
    pub catalog: Vec<u8>,
    pub files: HashMap<String, Vec<u8>>,
    /// When set, `fetch` fails (used to prove plan never downloads).
    pub forbid_fetch: bool,
}

impl MemoryServer {
    /// A source that answers only the version and catalog, from a saved file.
    pub fn from_saved_catalog(version: String, catalog: Vec<u8>) -> Self {
        MemoryServer { version, catalog, files: HashMap::new(), forbid_fetch: true }
    }
}

impl Source for MemoryServer {
    fn open(&self) -> Result<Box<dyn Session>> {
        Ok(Box::new(self.clone()))
    }
    fn describe(&self) -> String {
        "saved catalog".into()
    }
}

impl Session for MemoryServer {
    fn version(&mut self) -> Result<ServerVersion> {
        Ok(ServerVersion {
            release_unix: 0,
            flags: 0,
            label: "offline".into(),
            seen_from: String::new(),
            version: self.version.clone(),
        })
    }
    fn catalog(&mut self) -> Result<Vec<u8>> {
        Ok(self.catalog.clone())
    }
    fn fetch(&mut self, path: &str, len: u64, on_bytes: &mut dyn FnMut(u64)) -> Result<Vec<u8>> {
        if self.forbid_fetch {
            return Err(PatchError::failed(format!("{path}: this source does not download files")));
        }
        let data = self.files.get(path).ok_or_else(|| PatchError::failed(format!("{path}: not on the server")))?;
        if data.len() as u64 != len {
            return Err(PatchError::failed(format!("{path}: server has {} bytes, catalog says {len}", data.len())));
        }
        on_bytes(len);
        Ok(data.clone())
    }
}

#[cfg(any(test, feature = "fake-server"))]
#[allow(dead_code)]
pub(crate) mod loopback {
    //! A tiny patch server on 127.0.0.1 that speaks the real frames, so the
    //! TCP client is tested without leaving the machine.
    use super::*;
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::thread;

    pub fn serve(version: &str, catalog_text: &[u8], files: HashMap<String, Vec<u8>>) -> LiveServer {
        serve_at("127.0.0.1:0", Duration::ZERO, version, catalog_text, files)
    }

    /// Like [`serve`], on a chosen loopback address, sleeping `delay` before each file chunk.
    pub fn serve_at(
        addr: &str,
        delay: Duration,
        version: &str,
        catalog_text: &[u8],
        files: HashMap<String, Vec<u8>>,
    ) -> LiveServer {
        let listener = TcpListener::bind(addr).unwrap();
        let port = listener.local_addr().unwrap().port();
        let version = version.to_string();
        let coded = codec::testkit::slc(catalog_text);
        let files = Arc::new(files);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { return };
                let (version, coded, files) = (version.clone(), coded.clone(), files.clone());
                thread::spawn(move || {
                    loop {
                        let mut head = [0u8; 4];
                        if s.read_exact(&mut head).is_err() {
                            return;
                        }
                        let total = u32::from_le_bytes(head) as usize;
                        let mut f = vec![0u8; total];
                        f[..4].copy_from_slice(&head);
                        if s.read_exact(&mut f[4..]).is_err() {
                            return;
                        }
                        let Ok(req) = wire::decode(&f) else {
                            let _ = s.write_all(&wire::encode(wire::op::REFUSED, &[]));
                            continue;
                        };
                        let p = req.payload;
                        let reply = match req.operation {
                            wire::op::ASK_VERSION if &p[..8] == b"W20\x000001" => {
                                let mut b = Vec::new();
                                b.extend_from_slice(&1_800_000_000u32.to_le_bytes());
                                b.extend_from_slice(&1u32.to_le_bytes());
                                let mut field = [0u8; wire::VERSION_FIELD];
                                field[..5].copy_from_slice(b"empty");
                                field[6..15].copy_from_slice(b"127.0.0.1");
                                b.extend_from_slice(&field);
                                b.extend_from_slice(&(version.len() as u32 + 1).to_le_bytes());
                                b.extend_from_slice(version.as_bytes());
                                b.push(0);
                                wire::encode(wire::op::VERSION, &b)
                            }
                            wire::op::HELLO if p == b"W20\x000001" => {
                                let mut b = vec![wire::CATALOG_READY];
                                b.extend_from_slice(&coded);
                                wire::encode(wire::op::CATALOG, &b)
                            }
                            wire::op::READ => {
                                let off = u32::from_le_bytes(p[0..4].try_into().unwrap()) as usize;
                                let len = u32::from_le_bytes(p[4..8].try_into().unwrap()) as usize;
                                let name_len = u32::from_le_bytes(p[16..20].try_into().unwrap()) as usize;
                                let name = String::from_utf8_lossy(&p[20..20 + name_len - 1]).into_owned();
                                if !delay.is_zero() {
                                    thread::sleep(delay);
                                }
                                match files.get(&name) {
                                    Some(data) if off < data.len() => {
                                        let piece = &data[off..(off + len).min(data.len())];
                                        let mut b = Vec::new();
                                        b.extend_from_slice(&(off as u32).to_le_bytes());
                                        b.extend_from_slice(&(piece.len() as u32).to_le_bytes());
                                        b.extend_from_slice(&(name_len as u32).to_le_bytes());
                                        b.extend_from_slice(&p[20..20 + name_len]);
                                        b.extend_from_slice(piece);
                                        wire::encode(wire::op::CHUNK, &b)
                                    }
                                    _ => wire::encode(wire::op::REFUSED, &[]),
                                }
                            }
                            _ => wire::encode(wire::op::REFUSED, &[]),
                        };
                        if s.write_all(&reply).is_err() {
                            return;
                        }
                    }
                });
            }
        });
        LiveServer { host: "127.0.0.1".into(), port, ident: Ident::PC_FFXI, timeout: Duration::from_secs(10) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tcp_client_talks_to_a_local_server() {
        let mut files = HashMap::new();
        let big: Vec<u8> = (0..200_000u32).map(|i| (i % 241) as u8).collect();
        files.insert("s/Direct/big.slc".to_string(), big.clone());
        let server = loopback::serve("30261101_0", b"file a {\n}\nend\n", files);
        let mut s = server.open().unwrap();
        let v = s.version().unwrap();
        assert_eq!(v.version, "30261101_0");
        assert_eq!(v.label, "empty");
        assert_eq!(s.catalog().unwrap(), b"file a {\n}\nend\n");
        let mut seen = Vec::new();
        let got = s.fetch("s/Direct/big.slc", big.len() as u64, &mut |n| seen.push(n)).unwrap();
        assert_eq!(got, big);
        assert_eq!(seen, vec![65536, 131072, 196608, 200000]);
        let err = s.fetch("missing.slc", 10, &mut |_| {}).unwrap_err().to_string();
        assert!(err.contains("refused"), "{err}");
    }

    #[test]
    fn memory_server_checks_lengths_and_can_forbid_downloads() {
        let mut m = MemoryServer::default();
        m.files.insert("x".into(), vec![1, 2, 3]);
        assert_eq!(m.fetch("x", 3, &mut |_| {}).unwrap(), [1, 2, 3]);
        assert!(m.fetch("x", 4, &mut |_| {}).is_err());
        let mut saved = MemoryServer::from_saved_catalog("v".into(), b"end\n".to_vec());
        assert!(saved.fetch("x", 3, &mut |_| {}).is_err());
    }
}
