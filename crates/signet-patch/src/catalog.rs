//! The file catalog: the server's manifest, the same text PlayOnline keeps
//! as `FINAL FANTASY XI\patch.cfg`.
//!
//! ```text
//! file <path> {
//! <stamp> <size> <byte sum> <md5 head> <stamp>/Direct/<path>.slc <len> [<stamp>/Indirect/<path>.olc <len>]
//! ...one line per version, oldest first; the last line is current...
//! }
//! end
//! ```
//!
//! Paths may contain spaces. Each version line carries a signature: size,
//! the sum of the bytes taken as signed 8-bit values (wrapping i32), and the
//! first four bytes of the file's MD5 read as a little-endian i32.

use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use md5::{Digest, Md5};

use crate::error::{PatchError, Result, io_at};

pub const LOCAL_CATALOG: &str = "patch.cfg";

/// The signature a catalog line asserts for one version of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Signature {
    pub size: u64,
    pub byte_sum: i32,
    pub md5_head: i32,
}

#[derive(Default)]
struct SigBuilder {
    md5: Md5,
    sum: i32,
    size: u64,
}

impl SigBuilder {
    fn feed(&mut self, data: &[u8]) {
        let mut sum = self.sum;
        for &b in data {
            sum = sum.wrapping_add(b as i8 as i32);
        }
        self.sum = sum;
        self.size += data.len() as u64;
        self.md5.update(data);
    }
    fn done(self) -> Signature {
        let d = self.md5.finalize();
        Signature { size: self.size, byte_sum: self.sum, md5_head: i32::from_le_bytes([d[0], d[1], d[2], d[3]]) }
    }
}

impl Signature {
    pub fn of_bytes(data: &[u8]) -> Signature {
        let mut b = SigBuilder::default();
        b.feed(data);
        b.done()
    }

    /// Stream a file from disk (read-only).
    pub fn of_file(path: &Path) -> Result<Signature> {
        let mut f = io_at(File::open(path), path)?;
        let mut b = SigBuilder::default();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = io_at(f.read(&mut buf), path)?;
            if n == 0 {
                break;
            }
            b.feed(&buf[..n]);
        }
        Ok(b.done())
    }
}

/// A file on the server: its path and declared length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blob {
    pub path: String,
    pub len: u64,
}

/// One version line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    pub stamp: String,
    pub sig: Signature,
    /// The whole file at this version.
    pub image: Blob,
    /// Delta from the previous line's version to this one, if the server has it.
    pub delta: Option<Blob>,
}

/// One `file` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    /// Oldest first; never empty.
    pub revisions: Vec<Revision>,
}

impl Entry {
    pub fn current(&self) -> &Revision {
        self.revisions.last().expect("parse drops entries without revisions")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalog {
    pub entries: Vec<Entry>,
}

impl Catalog {
    /// Highest version stamp anywhere in the catalog.
    pub fn newest_stamp(&self) -> Option<&str> {
        self.entries.iter().flat_map(|e| e.revisions.iter().map(|r| r.stamp.as_str())).max()
    }
}

/// Take a server path ending in `suffix` off the front of `rest`.
fn take_path<'a>(rest: &'a str, suffix: &str) -> Option<(&'a str, &'a str)> {
    let marker = format!("{suffix} ");
    let cut = rest.find(&marker)? + suffix.len();
    Some((&rest[..cut], &rest[cut + 1..]))
}

fn take_number(rest: &str) -> Option<(u64, &str)> {
    let (tok, rest) = rest.split_once(' ').unwrap_or((rest, ""));
    Some((tok.parse().ok()?, rest))
}

fn parse_revision(line: &str) -> Option<Revision> {
    let mut head = line.splitn(5, ' ');
    let stamp = head.next()?.to_string();
    let size = head.next()?.parse().ok()?;
    let byte_sum = head.next()?.parse().ok()?;
    let md5_head = head.next()?.parse().ok()?;
    let rest = head.next()?;
    let (image_path, rest) = take_path(rest, ".slc")?;
    let (image_len, rest) = take_number(rest)?;
    let delta = if rest.is_empty() {
        None
    } else {
        let (delta_path, rest) = take_path(rest, ".olc")?;
        let (delta_len, rest) = take_number(rest)?;
        if !rest.is_empty() {
            return None;
        }
        Some(Blob { path: delta_path.to_string(), len: delta_len })
    };
    Some(Revision {
        stamp,
        sig: Signature { size, byte_sum, md5_head },
        image: Blob { path: image_path.to_string(), len: image_len },
        delta,
    })
}

pub fn parse(text: &str) -> Result<Catalog> {
    let mut entries = Vec::new();
    let mut open: Option<Entry> = None;
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let bad = |what: &str| PatchError::failed(format!("catalog line {}: {what}: {line:?}", n + 1));
        match open.as_mut() {
            None if line == "end" => break,
            None => {
                let path = line
                    .strip_prefix("file ")
                    .and_then(|s| s.strip_suffix(" {"))
                    .ok_or_else(|| bad("expected a file block"))?;
                check_relative(path).map_err(|e| e.context(format!("catalog line {}", n + 1)))?;
                open = Some(Entry { path: path.to_string(), revisions: Vec::new() });
            }
            Some(_) if line == "}" => {
                let entry = open.take().expect("checked above");
                if !entry.revisions.is_empty() {
                    entries.push(entry);
                }
            }
            Some(entry) => entry.revisions.push(parse_revision(line).ok_or_else(|| bad("bad version line"))?),
        }
    }
    if open.is_some() {
        return Err(PatchError::failed("catalog ends inside a file block"));
    }
    Ok(Catalog { entries })
}

/// Parse catalog bytes. The catalog is plain ASCII; anything else is refused
/// rather than guessed at, since its paths become file names.
pub fn parse_bytes(bytes: &[u8]) -> Result<Catalog> {
    let text = std::str::from_utf8(bytes).map_err(|e| PatchError::failed(format!("catalog is not text: {e}")))?;
    parse(text)
}

/// A catalog path must stay inside the install: relative, no `..`, no drive.
pub fn check_relative(path: &str) -> Result<PathBuf> {
    let p = Path::new(path);
    let ok = !path.is_empty()
        && !path.contains('\0')
        && !path.contains(':')
        && !path.starts_with('/')
        && !path.starts_with('\\')
        && p.components().all(|c| matches!(c, Component::Normal(_)));
    if ok { Ok(p.to_path_buf()) } else { Err(PatchError::refused(format!("unsafe path in catalog: {path:?}"))) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "file VTABLE.DAT {\n\
        30100101_0 10 20 30 30100101_0/Direct/VTABLE.DAT.slc 7\n\
        30200101_0 11 21 -31 30200101_0/Direct/VTABLE.DAT.slc 8 30200101_0/Indirect/VTABLE.DAT.olc 3\n\
        }\n\
        \r\n\
        file Tools/My Tool Help.chm {\r\n\
        30200101_1 5 -5 99 30200101_1/Direct/Tools/My Tool Help.chm.slc 4\r\n\
        }\r\n\
        file nothing.dat {\n\
        }\n\
        end\n\
        file after/end.dat {\n";

    #[test]
    fn parses_blocks_spaces_and_end() {
        let c = parse(SAMPLE).unwrap();
        assert_eq!(c.entries.len(), 2);
        let v = &c.entries[0];
        assert_eq!(v.path, "VTABLE.DAT");
        assert_eq!(v.revisions.len(), 2);
        assert_eq!(v.revisions[0].delta, None);
        let cur = v.current();
        assert_eq!(cur.stamp, "30200101_0");
        assert_eq!(cur.sig, Signature { size: 11, byte_sum: 21, md5_head: -31 });
        assert_eq!(cur.image, Blob { path: "30200101_0/Direct/VTABLE.DAT.slc".into(), len: 8 });
        assert_eq!(cur.delta, Some(Blob { path: "30200101_0/Indirect/VTABLE.DAT.olc".into(), len: 3 }));
        let t = &c.entries[1];
        assert_eq!(t.path, "Tools/My Tool Help.chm");
        assert_eq!(t.current().image.path, "30200101_1/Direct/Tools/My Tool Help.chm.slc");
        assert_eq!(c.newest_stamp(), Some("30200101_1"));
    }

    #[test]
    fn rejects_malformed_text() {
        assert!(parse("file a {\nnot a version\n}\n").is_err());
        assert!(parse("file a {\n").is_err());
        assert!(parse("30100101_0 1 2 3 a.slc 4\n").is_err());
        assert!(parse("file a {\n30100101_0 1 2 3 a.slc 4 a.olc 5 more\n}\n").is_err());
        assert!(parse("file a {\n30100101_0 x 2 3 a.slc 4\n}\n").is_err());
        assert!(parse_bytes(&[b'f', 0xff, 0xfe]).is_err());
    }

    #[test]
    fn refuses_paths_that_leave_the_install() {
        for bad in ["../x.dat", "ROM/../../x", "/abs", "\\abs", "C:/x", "C:x", "", "a\0b"] {
            assert!(check_relative(bad).is_err(), "{bad:?}");
            assert!(parse(&format!("file {bad} {{\n}}\n")).is_err() || bad.is_empty(), "{bad:?}");
        }
        assert!(check_relative("ROM/0/0.DAT").is_ok());
        assert!(check_relative("Tools/FINAL FANTASY XI Config.chm").is_ok());
    }

    #[test]
    fn signature_matches_independent_values() {
        // Expected values computed outside Rust (Python hashlib).
        assert_eq!(Signature::of_bytes(b"hello world"), Signature { size: 11, byte_sum: 1116, md5_head: -1153714594 });
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(Signature::of_bytes(&all), Signature { size: 256, byte_sum: -128, md5_head: -614086430 });
    }

    #[test]
    fn signature_from_disk_equals_from_memory() {
        let dir = crate::testdir("sig");
        let data: Vec<u8> = (0..2_500_000u32).map(|i| (i * 13 % 253) as u8).collect();
        let p = dir.join("f.bin");
        std::fs::write(&p, &data).unwrap();
        assert_eq!(Signature::of_file(&p).unwrap(), Signature::of_bytes(&data));
        std::fs::remove_dir_all(&dir).ok();
    }
}
