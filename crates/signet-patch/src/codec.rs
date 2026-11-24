//! The patch server's two codings, plus the flag byte on whole-file downloads.
//!
//! Both codings are a stream of items read LSB-first from a bit buffer that
//! starts with a u32 LE count of meaningful bits.
//!
//! `.slc` (a whole file):
//! - `0`, then 8 bits: one literal byte.
//! - `1`, then 16 bits distance, 8 bits length: copy from the output already
//!   written, `distance` bytes back (overlap allowed, byte by byte).
//!
//! `.olc` (a delta against the previous version of the same file):
//! - `0`, then 8 bits: literal byte.
//! - `1 0`, then 16 bits distance, 8 bits length: copy from the output, as above.
//! - `1 1`, then 16 bits signed offset, 8 bits length: copy from the old file,
//!   starting at (current output length + offset).

use crate::error::{PatchError, Result};

/// Flag bytes in front of a whole-file download.
pub mod flag {
    pub const STORED: u8 = 1;
    pub const SLC: u8 = 2;
    pub const ZLIB: u8 = 3;
}

struct Bits<'a> {
    bytes: &'a [u8],
    at: usize,
    end: usize,
}

impl<'a> Bits<'a> {
    fn new(coded: &'a [u8]) -> Result<Self> {
        let Some(head) = coded.get(..4) else {
            return Err(PatchError::failed("coded stream has no 4-byte bit count"));
        };
        let end = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
        let bytes = &coded[4..];
        if end > bytes.len() * 8 {
            return Err(PatchError::failed(format!(
                "coded stream says {end} bits but carries only {}",
                bytes.len() * 8
            )));
        }
        Ok(Bits { bytes, at: 0, end })
    }

    fn more(&self) -> bool {
        self.at < self.end
    }

    /// Read `n` (at most 16) bits, first bit read becomes bit 0.
    fn take(&mut self, n: u32) -> Result<u32> {
        debug_assert!(n <= 16);
        if self.at + n as usize > self.end {
            return Err(PatchError::failed(format!("coded stream ends inside an item at bit {}", self.at)));
        }
        let first = self.at >> 3;
        let mut window = 0u32;
        for i in 0..3 {
            if let Some(&b) = self.bytes.get(first + i) {
                window |= (b as u32) << (8 * i);
            }
        }
        let v = (window >> (self.at & 7)) & ((1u32 << n) - 1);
        self.at += n as usize;
        Ok(v)
    }
}

fn grow(out: &[u8], add: usize, limit: usize) -> Result<()> {
    if out.len() + add > limit {
        return Err(PatchError::failed(format!("decoded data would pass the {limit}-byte limit")));
    }
    Ok(())
}

fn copy_from_output(out: &mut Vec<u8>, distance: usize, len: usize, limit: usize) -> Result<()> {
    if distance == 0 || distance > out.len() {
        return Err(PatchError::failed(format!(
            "copy reaches {distance} bytes back with only {} written",
            out.len()
        )));
    }
    grow(out, len, limit)?;
    let from = out.len() - distance;
    for i in 0..len {
        let b = out[from + i];
        out.push(b);
    }
    Ok(())
}

/// Decode a `.slc` stream. `limit` caps the output size.
pub fn expand_slc(coded: &[u8], limit: usize) -> Result<Vec<u8>> {
    let mut bits = Bits::new(coded)?;
    let mut out = Vec::with_capacity(coded.len().saturating_mul(2).min(limit));
    while bits.more() {
        if bits.take(1)? == 0 {
            grow(&out, 1, limit)?;
            out.push(bits.take(8)? as u8);
        } else {
            let distance = bits.take(16)? as usize;
            let len = bits.take(8)? as usize;
            copy_from_output(&mut out, distance, len, limit)?;
        }
    }
    Ok(out)
}

/// Apply a `.olc` delta to `old`. `limit` caps the output size.
pub fn apply_olc(delta: &[u8], old: &[u8], limit: usize) -> Result<Vec<u8>> {
    let mut bits = Bits::new(delta)?;
    let mut out = Vec::with_capacity(old.len().min(limit));
    while bits.more() {
        if bits.take(1)? == 0 {
            grow(&out, 1, limit)?;
            out.push(bits.take(8)? as u8);
        } else if bits.take(1)? == 0 {
            let distance = bits.take(16)? as usize;
            let len = bits.take(8)? as usize;
            copy_from_output(&mut out, distance, len, limit)?;
        } else {
            let offset = bits.take(16)? as u16 as i16 as i64;
            let len = bits.take(8)? as usize;
            let start = out.len() as i64 + offset;
            let src = usize::try_from(start)
                .ok()
                .and_then(|s| old.get(s..s + len))
                .ok_or_else(|| {
                    PatchError::failed(format!(
                        "delta copies old bytes {start}..{} of a {}-byte file",
                        start + len as i64,
                        old.len()
                    ))
                })?;
            grow(&out, len, limit)?;
            out.extend_from_slice(src);
        }
    }
    Ok(out)
}

/// Unpack a whole-file download: flag byte, then stored, `.slc` or zlib data.
pub fn unpack_download(raw: &[u8], limit: usize) -> Result<Vec<u8>> {
    let Some((&kind, body)) = raw.split_first() else {
        return Err(PatchError::failed("download is empty"));
    };
    match kind {
        flag::STORED => {
            grow(&[], body.len(), limit)?;
            Ok(body.to_vec())
        }
        flag::SLC => expand_slc(body, limit),
        flag::ZLIB => miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(body, limit)
            .map_err(|e| PatchError::failed(format!("zlib data is bad: {:?}", e.status))),
        other => Err(PatchError::failed(format!("download starts with unknown coding flag {other}"))),
    }
}

/// Encoders used only to build test fixtures. They make valid streams, not
/// small ones.
#[cfg(any(test, feature = "fake-server"))]
#[allow(dead_code)]
pub(crate) mod testkit {
    pub struct BitSink {
        bits: Vec<bool>,
    }

    impl BitSink {
        pub fn new() -> Self {
            BitSink { bits: Vec::new() }
        }
        pub fn put(&mut self, value: u32, n: u32) {
            for i in 0..n {
                self.bits.push((value >> i) & 1 == 1);
            }
        }
        pub fn finish(&self) -> Vec<u8> {
            let mut out = (self.bits.len() as u32).to_le_bytes().to_vec();
            for chunk in self.bits.chunks(8) {
                out.push(chunk.iter().enumerate().fold(0u8, |b, (i, &on)| b | ((on as u8) << i)));
            }
            out
        }
    }

    /// Greedy `.slc` encoder: back-references to the previous 64 bytes, else literals.
    pub fn slc(data: &[u8]) -> Vec<u8> {
        let mut s = BitSink::new();
        let mut i = 0;
        while i < data.len() {
            let mut best = (0usize, 0usize);
            for d in 1..=i.min(64) {
                let mut n = 0;
                while i + n < data.len() && n < 255 && data[i + n] == data[i + n - d] {
                    n += 1;
                }
                if n > best.1 {
                    best = (d, n);
                }
            }
            if best.1 >= 3 {
                s.put(1, 1);
                s.put(best.0 as u32, 16);
                s.put(best.1 as u32, 8);
                i += best.1;
            } else {
                s.put(0, 1);
                s.put(data[i] as u32, 8);
                i += 1;
            }
        }
        s.finish()
    }

    /// `.olc` encoder: copies from the old file at the same position where it
    /// matches, otherwise literals.
    pub fn olc(old: &[u8], new: &[u8]) -> Vec<u8> {
        let mut s = BitSink::new();
        let mut i = 0;
        while i < new.len() {
            let mut n = 0;
            while i + n < new.len() && i + n < old.len() && n < 255 && new[i + n] == old[i + n] {
                n += 1;
            }
            if n >= 3 {
                s.put(0b11, 2);
                s.put(0, 16);
                s.put(n as u32, 8);
                i += n;
            } else {
                s.put(0, 1);
                s.put(new[i] as u32, 8);
                i += 1;
            }
        }
        s.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::testkit::*;
    use super::*;

    const BIG: usize = 1 << 24;

    #[test]
    fn slc_literals_and_overlapping_copy() {
        let mut s = BitSink::new();
        for &c in b"xyz" {
            s.put(0, 1);
            s.put(c as u32, 8);
        }
        s.put(1, 1);
        s.put(3, 16);
        s.put(7, 8);
        assert_eq!(expand_slc(&s.finish(), BIG).unwrap(), b"xyzxyzxyzx");
    }

    #[test]
    fn slc_round_trips_mixed_data() {
        let mut data = Vec::new();
        for i in 0..5000u32 {
            data.push((i % 7) as u8);
            if i % 13 == 0 {
                data.extend_from_slice(&i.to_le_bytes());
            }
        }
        assert_eq!(expand_slc(&slc(&data), BIG).unwrap(), data);
        assert_eq!(expand_slc(&slc(&[]), BIG).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn slc_rejects_bad_streams() {
        let mut s = BitSink::new();
        s.put(1, 1);
        s.put(1, 16);
        s.put(1, 8);
        assert!(expand_slc(&s.finish(), BIG).unwrap_err().to_string().contains("back"));
        let mut lying = slc(b"abc");
        lying[0] = 200;
        assert!(expand_slc(&lying, BIG).unwrap_err().to_string().contains("carries"));
        assert!(expand_slc(&[1, 0], BIG).is_err());
        // Bit count stops in the middle of a literal.
        let mut cut = slc(b"abc");
        cut[0] = 13;
        assert!(expand_slc(&cut, BIG).unwrap_err().to_string().contains("inside"));
        assert!(expand_slc(&slc(&[5u8; 1000]), 100).unwrap_err().to_string().contains("limit"));
    }

    #[test]
    fn olc_uses_all_three_items() {
        let old = b"ABCDEFGHIJKLMNOP";
        let mut s = BitSink::new();
        s.put(0b11, 2); // old copy at +2, 4 bytes: CDEF
        s.put(2, 16);
        s.put(4, 8);
        s.put(0, 1); // literal '!'
        s.put(b'!' as u32, 8);
        s.put(0b11, 2); // old copy at 5 + (-5) = 0, 3 bytes: ABC
        s.put((-5i16) as u16 as u32, 16);
        s.put(3, 8);
        s.put(0b01, 2); // output copy, 4 back, 6 bytes (overlapping)
        s.put(4, 16);
        s.put(6, 8);
        assert_eq!(apply_olc(&s.finish(), old, BIG).unwrap(), b"CDEF!ABC!ABC!A");
    }

    #[test]
    fn olc_round_trips_an_edit() {
        let old: Vec<u8> = (0..4000u32).map(|i| (i * 31 % 251) as u8).collect();
        let mut new = old.clone();
        new[100..110].copy_from_slice(b"0123456789");
        new.truncate(3500);
        new.extend_from_slice(b"tail added");
        assert_eq!(apply_olc(&olc(&old, &new), &old, BIG).unwrap(), new);
    }

    #[test]
    fn olc_rejects_reads_outside_the_old_file() {
        let mut s = BitSink::new();
        s.put(0b11, 2);
        s.put((-1i16) as u16 as u32, 16);
        s.put(2, 8);
        assert!(apply_olc(&s.finish(), b"abc", BIG).unwrap_err().to_string().contains("old bytes"));
        let mut s = BitSink::new();
        s.put(0b11, 2);
        s.put(2, 16);
        s.put(5, 8);
        assert!(apply_olc(&s.finish(), b"abc", BIG).is_err());
    }

    #[test]
    fn download_flags() {
        assert_eq!(unpack_download(&[flag::STORED, 1, 2, 3], BIG).unwrap(), [1, 2, 3]);
        let mut coded = vec![flag::SLC];
        coded.extend(slc(b"hello hello hello"));
        assert_eq!(unpack_download(&coded, BIG).unwrap(), b"hello hello hello");
        let mut z = vec![flag::ZLIB];
        z.extend(miniz_oxide::deflate::compress_to_vec_zlib(b"zlib body zlib body", 6));
        assert_eq!(unpack_download(&z, BIG).unwrap(), b"zlib body zlib body");
        let mut bad = z.clone();
        let n = bad.len();
        bad[n - 1] ^= 0xff;
        assert!(unpack_download(&bad, BIG).is_err());
        assert!(unpack_download(&[9, 1], BIG).unwrap_err().to_string().contains("unknown"));
        assert!(unpack_download(&[], BIG).is_err());
    }
}
