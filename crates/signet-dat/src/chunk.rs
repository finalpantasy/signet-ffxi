//! Resource chunk stream inside a DAT file.
//!
//! A DAT is a run of chunks. Each chunk starts with a 16-byte header:
//! a 4-byte name, a little-endian u32 whose low 7 bits are the chunk kind and
//! whose next 19 bits are the chunk length in 16-byte units (header included),
//! then 8 reserved bytes.
//!
//! Many DATs are fixed-size containers: the chunk stream ends at a header whose
//! name and kind/length word are all zero, and the rest of the file is padding
//! or unused space. The iterator stops cleanly there; see [`Chunks::stream_end`].

use std::fmt;

pub const HEADER_LEN: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Chunk<'a> {
    pub name: [u8; 4],
    pub kind: u8,
    /// Offset of the header within the DAT.
    pub offset: usize,
    /// Payload after the 16-byte header.
    pub data: &'a [u8],
}

impl Chunk<'_> {
    pub fn name_str(&self) -> String {
        self.name
            .iter()
            .map(|&b| if b.is_ascii_graphic() { b as char } else { '.' })
            .collect()
    }
}

impl fmt::Debug for Chunk<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Chunk({:?} kind=0x{:02x} at={} len={})",
            self.name_str(),
            self.kind,
            self.offset,
            self.data.len()
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkError {
    /// Fewer than 16 bytes left but not at the end.
    TruncatedHeader { offset: usize, remaining: usize },
    /// Declared length is below the header size.
    TooShort { offset: usize, len: usize },
    /// Declared length runs past the end of the file.
    PastEnd { offset: usize, len: usize, file_len: usize },
}

impl fmt::Display for ChunkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TruncatedHeader { offset, remaining } => {
                write!(f, "truncated chunk header at {offset} ({remaining} bytes left)")
            }
            Self::TooShort { offset, len } => write!(f, "chunk at {offset} declares length {len}"),
            Self::PastEnd { offset, len, file_len } => {
                write!(f, "chunk at {offset} length {len} runs past end {file_len}")
            }
        }
    }
}

impl std::error::Error for ChunkError {}

/// Iterates the chunks of a DAT. Stops after the first error.
pub fn chunks(dat: &[u8]) -> Chunks<'_> {
    Chunks { dat, pos: 0, failed: false, ended: false }
}

pub struct Chunks<'a> {
    dat: &'a [u8],
    pos: usize,
    failed: bool,
    ended: bool,
}

impl Chunks<'_> {
    /// Where the chunk stream ended once iteration finished cleanly: the file
    /// length, or the offset of the zero end header. `None` after an error or
    /// before iteration is complete.
    pub fn stream_end(&self) -> Option<usize> {
        (!self.failed && (self.ended || self.pos >= self.dat.len())).then_some(self.pos)
    }
}

impl<'a> Iterator for Chunks<'a> {
    type Item = Result<Chunk<'a>, ChunkError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.ended || self.pos >= self.dat.len() {
            return None;
        }
        let offset = self.pos;
        let remaining = self.dat.len() - offset;
        let fail = |s: &mut Self, e| {
            s.failed = true;
            Some(Err(e))
        };
        if remaining < HEADER_LEN {
            return fail(self, ChunkError::TruncatedHeader { offset, remaining });
        }
        let h = &self.dat[offset..offset + HEADER_LEN];
        let info = u32::from_le_bytes([h[4], h[5], h[6], h[7]]);
        if info == 0 && h[..4] == [0; 4] {
            self.ended = true;
            return None;
        }
        let kind = (info & 0x7f) as u8;
        let len = ((info >> 7) & 0x7_ffff) as usize * 16;
        if len < HEADER_LEN {
            return fail(self, ChunkError::TooShort { offset, len });
        }
        if len > remaining {
            return fail(self, ChunkError::PastEnd { offset, len, file_len: self.dat.len() });
        }
        self.pos = offset + len;
        Some(Ok(Chunk {
            name: [h[0], h[1], h[2], h[3]],
            kind,
            offset,
            data: &self.dat[offset + HEADER_LEN..offset + len],
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(name: &[u8; 4], kind: u8, units: u32) -> Vec<u8> {
        let mut v = name.to_vec();
        v.extend_from_slice(&((units << 7) | kind as u32).to_le_bytes());
        v.extend_from_slice(&[0; 8]);
        v
    }

    #[test]
    fn walks_two_chunks() {
        let mut dat = header(b"abcd", 0x20, 2);
        dat.extend_from_slice(&[7; 16]);
        dat.extend(header(b"wxyz", 0x01, 1));
        let got: Vec<_> = chunks(&dat).collect::<Result<_, _>>().unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].kind, 0x20);
        assert_eq!(got[0].data, &[7; 16]);
        assert_eq!(got[1].offset, 32);
        assert!(got[1].data.is_empty());
    }

    #[test]
    fn stops_at_zero_header() {
        let mut dat = header(b"abcd", 0x20, 1);
        dat.extend_from_slice(&[0; 48]);
        let mut it = chunks(&dat);
        assert_eq!(it.by_ref().count(), 1);
        assert_eq!(it.stream_end(), Some(16));
    }

    #[test]
    fn rejects_overrun() {
        let dat = header(b"abcd", 0x20, 3);
        let got: Vec<_> = chunks(&dat).collect();
        assert!(matches!(got[0], Err(ChunkError::PastEnd { .. })));
        assert_eq!(got.len(), 1);
    }
}
