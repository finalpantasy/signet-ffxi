//! The patch server's TCP framing.
//!
//! Every message, both ways, is one frame:
//!
//! ```text
//! +0  u32 LE  total frame length, header included
//! +4  [u8;4]  first four bytes of MD5 over bytes 8..end
//! +8  "POLP"
//! +12 u32 LE  operation
//! +16 ...     payload
//! ```
//!
//! Request payloads start (or, for reads, continue) with an 8-byte identity:
//! a platform tag and a title tag. Layouts checked against polcore.dll's
//! frame builders; see docs/REPLACEMENT_CLIENT.md for the source notes.

use md5::{Digest, Md5};

use crate::error::{PatchError, Result};

pub const MAGIC: [u8; 4] = *b"POLP";
pub const HEADER: usize = 16;
/// Largest slice one read request asks for.
pub const SLICE: u32 = 0x1_0000;
/// Fixed width of the version field in a version request and its reply.
pub const VERSION_FIELD: usize = 0x40;
/// Largest ordinary reply accepted (a full slice plus the echoed path).
pub const MAX_REPLY: usize = 0x1_6d00;
/// Largest catalog reply accepted. The catalog is one frame and runs to
/// megabytes, so it gets its own, much larger, ceiling.
pub const MAX_CATALOG_REPLY: usize = 1 << 30;
/// First payload byte of a good catalog reply.
pub const CATALOG_READY: u8 = 2;

/// Operation codes.
pub mod op {
    pub const HELLO: u32 = 1;
    pub const CATALOG: u32 = 2;
    pub const READ: u32 = 3;
    pub const CHUNK: u32 = 4;
    pub const REFUSED: u32 = 5;
    pub const BYE: u32 = 6;
    pub const ASK_VERSION: u32 = 7;
    pub const VERSION: u32 = 8;
}

/// Platform and title tags carried by client frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ident {
    pub platform: [u8; 4],
    pub title: [u8; 4],
}

impl Ident {
    /// Windows client, title 1 (FINAL FANTASY XI).
    pub const PC_FFXI: Ident = Ident { platform: *b"W20\0", title: *b"0001" };

    fn put(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.platform);
        out.extend_from_slice(&self.title);
    }
}

/// Where the patch server for a title lives: `pc%03d.pol.com`, port 53000 + title.
pub fn server_for_title(title: u16) -> (String, u16) {
    (format!("pc{title:03}.pol.com"), 53000 + title)
}

/// Frame check value: first four bytes of MD5 over everything after the check field.
pub fn check_value(after_check: &[u8]) -> [u8; 4] {
    let d = Md5::digest(after_check);
    [d[0], d[1], d[2], d[3]]
}

/// Build a complete frame around a payload.
pub fn encode(operation: u32, payload: &[u8]) -> Vec<u8> {
    let total = HEADER + payload.len();
    let mut f = Vec::with_capacity(total);
    f.extend_from_slice(&(total as u32).to_le_bytes());
    f.extend_from_slice(&[0; 4]);
    f.extend_from_slice(&MAGIC);
    f.extend_from_slice(&operation.to_le_bytes());
    f.extend_from_slice(payload);
    let check = check_value(&f[8..]);
    f[4..8].copy_from_slice(&check);
    f
}

/// A frame that passed length, magic and check-value tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame<'a> {
    pub operation: u32,
    pub payload: &'a [u8],
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

pub fn decode(bytes: &[u8]) -> Result<Frame<'_>> {
    if bytes.len() < HEADER {
        return Err(PatchError::failed(format!("frame of {} bytes is shorter than its header", bytes.len())));
    }
    let declared = le32(bytes, 0) as usize;
    if declared != bytes.len() {
        return Err(PatchError::failed(format!(
            "frame says {declared} bytes but {} arrived",
            bytes.len()
        )));
    }
    if bytes[8..12] != MAGIC {
        return Err(PatchError::failed(format!("frame magic is {:02x?}, not POLP", &bytes[8..12])));
    }
    if check_value(&bytes[8..]) != bytes[4..8] {
        return Err(PatchError::failed("frame check value does not match"));
    }
    Ok(Frame { operation: le32(bytes, 12), payload: &bytes[HEADER..] })
}

/// Hello: identity only. The server answers with the catalog.
pub fn hello(id: &Ident) -> Vec<u8> {
    let mut p = Vec::with_capacity(8);
    id.put(&mut p);
    encode(op::HELLO, &p)
}

/// Version question: identity, then a 64-byte NUL-padded local version.
pub fn ask_version(id: &Ident, local: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(8 + VERSION_FIELD);
    id.put(&mut p);
    let mut field = [0u8; VERSION_FIELD];
    let n = local.len().min(VERSION_FIELD - 1);
    field[..n].copy_from_slice(&local[..n]);
    p.extend_from_slice(&field);
    encode(op::ASK_VERSION, &p)
}

/// Read request: offset, length, identity, path length (with NUL), path, NUL.
pub fn read(id: &Ident, path: &str, offset: u32, len: u32) -> Vec<u8> {
    let mut p = Vec::with_capacity(21 + path.len());
    p.extend_from_slice(&offset.to_le_bytes());
    p.extend_from_slice(&len.to_le_bytes());
    id.put(&mut p);
    p.extend_from_slice(&(path.len() as u32 + 1).to_le_bytes());
    p.extend_from_slice(path.as_bytes());
    p.push(0);
    encode(op::READ, &p)
}

/// The server's answer to a version question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerVersion {
    pub release_unix: u32,
    pub flags: u32,
    /// "empty" for a blank local version; "unknown" when the server did not
    /// recognise what it was sent.
    pub label: String,
    /// The client address as the server saw it.
    pub seen_from: String,
    pub version: String,
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

/// Version reply payload: release time u32, flags u32, 64-byte field holding
/// two NUL-terminated strings (label, client address), u32 length, version text.
pub fn parse_version(payload: &[u8]) -> Result<ServerVersion> {
    let text_at = 8 + VERSION_FIELD + 4;
    if payload.len() < text_at {
        return Err(PatchError::failed(format!("version reply is only {} bytes", payload.len())));
    }
    let field = &payload[8..8 + VERSION_FIELD];
    let label_end = field.iter().position(|&c| c == 0).unwrap_or(field.len());
    let label = cstr(field);
    let seen_from = cstr(field.get(label_end + 1..).unwrap_or(&[]));
    let n = le32(payload, 8 + VERSION_FIELD) as usize;
    let text = payload
        .get(text_at..text_at.saturating_add(n))
        .ok_or_else(|| PatchError::failed("version reply text runs past the frame"))?;
    Ok(ServerVersion {
        release_unix: le32(payload, 0),
        flags: le32(payload, 4),
        label,
        seen_from,
        version: cstr(text),
    })
}

/// Chunk reply payload: offset u32, length u32, echo length u32, echo bytes, data.
/// Returns the offset and the data.
pub fn parse_chunk(payload: &[u8]) -> Result<(u32, &[u8])> {
    if payload.len() < 12 {
        return Err(PatchError::failed("chunk reply is shorter than its 12-byte header"));
    }
    let offset = le32(payload, 0);
    let len = le32(payload, 4) as usize;
    let echo = le32(payload, 8) as usize;
    let start = 12usize.saturating_add(echo);
    let data = payload.get(start..start.saturating_add(len)).ok_or_else(|| {
        PatchError::failed(format!(
            "chunk reply claims {len} data bytes after a {echo}-byte echo, but holds {}",
            payload.len()
        ))
    })?;
    Ok((offset, data))
}

/// Catalog reply payload: status byte, then the `.slc`-coded catalog text.
pub fn catalog_body(payload: &[u8]) -> Result<&[u8]> {
    match payload.split_first() {
        Some((&CATALOG_READY, rest)) => Ok(rest),
        Some((status, _)) => Err(PatchError::failed(format!("catalog reply status is {status}, not {CATALOG_READY}"))),
        None => Err(PatchError::failed("catalog reply is empty")),
    }
}

/// Hex of the first bytes of a frame, for error messages about odd replies.
pub fn hex_head(bytes: &[u8], max: usize) -> String {
    bytes.iter().take(max).map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_frame_matches_an_independent_md5() {
        // Expected bytes computed outside Rust (Python hashlib).
        let f = hello(&Ident::PC_FFXI);
        let want = [
            0x18, 0, 0, 0, 0xbf, 0x12, 0xa6, 0x01, b'P', b'O', b'L', b'P', 1, 0, 0, 0, b'W', b'2', b'0', 0, b'0', b'0',
            b'0', b'1',
        ];
        assert_eq!(f, want);
        let ps2 = Ident { platform: *b"PS2\0", title: *b"FFXI" };
        assert_eq!(&hello(&ps2)[4..8], &[0x62, 0x84, 0xa0, 0xa9]);
    }

    #[test]
    fn every_builder_round_trips() {
        let id = Ident::PC_FFXI;
        for (frame, want_op) in [
            (hello(&id), op::HELLO),
            (ask_version(&id, b"30190430_0"), op::ASK_VERSION),
            (read(&id, "30190430_0/Direct/ROM/0/0.DAT.slc", 0x20000, SLICE), op::READ),
            (encode(op::BYE, &[]), op::BYE),
        ] {
            let d = decode(&frame).unwrap();
            assert_eq!(d.operation, want_op);
            assert_eq!(d.payload, &frame[HEADER..]);
        }
    }

    #[test]
    fn version_question_is_fixed_width() {
        let f = ask_version(&Ident::PC_FFXI, b"30190430_0");
        assert_eq!(f.len(), 0x58);
        assert_eq!(&f[24..34], b"30190430_0");
        assert!(f[34..].iter().all(|&b| b == 0));
        let long = ask_version(&Ident::PC_FFXI, &[b'9'; 200]);
        assert_eq!(long.len(), 0x58);
        assert_eq!(*long.last().unwrap(), 0);
    }

    #[test]
    fn read_request_layout() {
        let f = read(&Ident::PC_FFXI, "a/b.slc", 7, 9);
        assert_eq!(f.len(), 0x24 + 8);
        let p = &f[HEADER..];
        assert_eq!(le32(p, 0), 7);
        assert_eq!(le32(p, 4), 9);
        assert_eq!(&p[8..16], b"W20\x000001");
        assert_eq!(le32(p, 16), 8);
        assert_eq!(&p[20..], b"a/b.slc\0");
    }

    #[test]
    fn decode_rejects_damage() {
        let good = read(&Ident::PC_FFXI, "x.slc", 0, 1);
        let mut flipped = good.clone();
        *flipped.last_mut().unwrap() ^= 0x40;
        assert!(decode(&flipped).unwrap_err().to_string().contains("check value"));
        let mut magic = good.clone();
        magic[10] = b'X';
        assert!(decode(&magic).unwrap_err().to_string().contains("magic"));
        assert!(decode(&good[..good.len() - 1]).unwrap_err().to_string().contains("arrived"));
        assert!(decode(&good[..10]).is_err());
    }

    #[test]
    fn version_reply_parses() {
        let mut p = Vec::new();
        p.extend_from_slice(&1_700_000_000u32.to_le_bytes());
        p.extend_from_slice(&1u32.to_le_bytes());
        let mut field = [0u8; VERSION_FIELD];
        field[..5].copy_from_slice(b"empty");
        field[6..15].copy_from_slice(b"192.0.2.9");
        p.extend_from_slice(&field);
        p.extend_from_slice(&11u32.to_le_bytes());
        p.extend_from_slice(b"30261101_0\0");
        let v = parse_version(&p).unwrap();
        assert_eq!(v.release_unix, 1_700_000_000);
        assert_eq!(v.flags, 1);
        assert_eq!(v.label, "empty");
        assert_eq!(v.seen_from, "192.0.2.9");
        assert_eq!(v.version, "30261101_0");
        assert!(parse_version(&p[..p.len() - 2]).is_err());
        assert!(parse_version(&p[..20]).is_err());
    }

    #[test]
    fn chunk_reply_skips_the_echo() {
        let mut p = Vec::new();
        p.extend_from_slice(&0x10000u32.to_le_bytes());
        p.extend_from_slice(&4u32.to_le_bytes());
        p.extend_from_slice(&6u32.to_le_bytes());
        p.extend_from_slice(b"f.slc\0DATA");
        assert_eq!(parse_chunk(&p).unwrap(), (0x10000, &b"DATA"[..]));
        assert!(parse_chunk(&p[..p.len() - 1]).is_err());
        assert!(parse_chunk(&p[..8]).is_err());
    }

    #[test]
    fn catalog_status_byte() {
        assert_eq!(catalog_body(&[2, 9, 9]).unwrap(), &[9, 9]);
        assert!(catalog_body(&[1, 9]).is_err());
        assert!(catalog_body(&[]).is_err());
    }

    #[test]
    fn ffxi_server_address() {
        assert_eq!(server_for_title(1), ("pc001.pol.com".to_string(), 53001));
    }
}
