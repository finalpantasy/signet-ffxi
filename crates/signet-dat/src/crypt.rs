//! Undoes the light obfuscation on zone chunks: MZB (kind 0x1C, zone layout) and
//! MMB (kind 0x2E, zone meshes).
//!
//! The two 256-byte key tables are not stored in Signet. They are read at runtime
//! from the user's own `FFXiMain.dll`, found by a byte-sum and FNV-1a fingerprint,
//! so Signet ships no client data.
//!
//! Layout of an obfuscated payload (after the 16-byte chunk header):
//! - bytes 0..3: decode length (u24 little-endian), counted from payload start
//! - byte 3: format version (MMB is obfuscated when >= 5, MZB when >= 0x1B)
//! - MZB bytes 4..7: placement record count (u24); byte 7 selects the key
//! - MMB byte 5 selects the key; bytes 6..8 == FF FF enable the block swap pass
//! - Some chunks are stored plain and start with the ASCII magic "MMB" or "MZB"
//!   (versions 1, 4, 8 and 9 seen) instead of a length.
//!
//! Format knowledge comes from public community work on FFXI zone files (see
//! docs/REPLACEMENT_CLIENT.md, "Sources"); this implementation is original.

use std::io;
use std::path::Path;

pub const MZB_KIND: u8 = 0x1c;
pub const MMB_KIND: u8 = 0x2e;
/// Size of one MZB placement record: 16-byte name, then 21 32-bit fields.
pub const MZB_RECORD: usize = 100;
const MZB_RECORDS_AT: usize = 32;

struct Fingerprint {
    sum: u32,
    fnv: u64,
}

const BYTE_KEYS: Fingerprint = Fingerprint { sum: 31091, fnv: 0x6dfe_650a_09e1_f80a };
const SWAP_KEYS: Fingerprint = Fingerprint { sum: 31638, fnv: 0x96ed_50ae_59dc_59f1 };

fn fnv1a(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &c| (h ^ c as u64).wrapping_mul(0x0100_0000_01b3))
}

/// Finds the 256-byte window matching `fp`, using a rolling sum then the hash.
fn find_table(image: &[u8], fp: &Fingerprint) -> Option<[u8; 256]> {
    if image.len() < 256 {
        return None;
    }
    let mut sum: u32 = image[..256].iter().map(|&b| b as u32).sum();
    for start in 0..=image.len() - 256 {
        if start > 0 {
            sum = sum - image[start - 1] as u32 + image[start + 255] as u32;
        }
        if sum == fp.sum && fnv1a(&image[start..start + 256]) == fp.fnv {
            return Some(image[start..start + 256].try_into().unwrap());
        }
    }
    None
}

/// Key tables read from the user's client.
pub struct ZoneKeys {
    byte_keys: [u8; 256],
    swap_keys: [u8; 256],
}

impl ZoneKeys {
    /// Reads both tables from `FFXiMain.dll` in the install root.
    pub fn from_install(root: &Path) -> io::Result<Self> {
        Self::from_client_image(&std::fs::read(root.join("FFXiMain.dll"))?)
    }

    pub fn from_client_image(image: &[u8]) -> io::Result<Self> {
        let missing = |what| io::Error::new(io::ErrorKind::NotFound, format!("{what} key table not found in client"));
        Ok(Self {
            byte_keys: find_table(image, &BYTE_KEYS).ok_or_else(|| missing("zone byte"))?,
            swap_keys: find_table(image, &SWAP_KEYS).ok_or_else(|| missing("zone swap"))?,
        })
    }

    #[cfg(test)]
    fn from_tables(byte_keys: [u8; 256], swap_keys: [u8; 256]) -> Self {
        Self { byte_keys, swap_keys }
    }
}

fn u24(p: &[u8], i: usize) -> usize {
    p[i] as usize | (p[i + 1] as usize) << 8 | (p[i + 2] as usize) << 16
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decoded {
    /// The payload was obfuscated and is now plain.
    Yes,
    /// The version byte says the payload was never obfuscated.
    AlreadyPlain,
    /// The declared length runs past the payload; nothing was changed.
    BadLength,
}

/// Decodes an MMB payload (chunk kind 0x2E) in place.
pub fn decode_mmb(p: &mut [u8], keys: &ZoneKeys) -> Decoded {
    if p.len() < 8 {
        return Decoded::BadLength;
    }
    if p.starts_with(b"MMB") {
        return Decoded::AlreadyPlain;
    }
    let len = u24(p, 0);
    if len > p.len() {
        return Decoded::BadLength;
    }
    let mut changed = false;
    if p[3] >= 5 {
        let mut key = keys.byte_keys[(p[5] ^ 0xf0) as usize] as u32;
        let mut step = 0u32;
        for b in &mut p[8..len] {
            let pair = (key & 0xff) << 8 | (key & 0xff);
            step += 1;
            key = key.wrapping_add(step);
            *b ^= (pair >> (key & 7)) as u8;
            step += 1;
            key = key.wrapping_add(step);
        }
        changed = true;
    }
    if p[6] == 0xff && p[7] == 0xff && len >= 8 {
        let mut k1 = (p[5] ^ 0xf0) as u32;
        let mut k2 = keys.swap_keys[k1 as usize] as u32;
        let half = ((len - 8) & !0xf) / 2;
        let (a, b) = p[8..8 + 2 * half].split_at_mut(half);
        for (x, y) in a.chunks_exact_mut(8).zip(b.chunks_exact_mut(8)) {
            if k2 & 1 != 0 {
                x.swap_with_slice(y);
            }
            k1 = k1.wrapping_add(9);
            k2 = k2.wrapping_add(k1);
        }
        changed = true;
    }
    if changed { Decoded::Yes } else { Decoded::AlreadyPlain }
}

/// Decodes an MZB payload (chunk kind 0x1C) in place, including record names.
pub fn decode_mzb(p: &mut [u8], keys: &ZoneKeys) -> Decoded {
    if p.len() < MZB_RECORDS_AT {
        return Decoded::BadLength;
    }
    if p[3] < 0x1b || p.starts_with(b"MZB") {
        return Decoded::AlreadyPlain;
    }
    let len = u24(p, 0);
    if len > p.len() {
        return Decoded::BadLength;
    }
    let mut key = keys.byte_keys[(p[7] ^ 0xff) as usize] as u32;
    let mut step = 0u32;
    let mut pos = 8;
    while pos < len {
        let run = ((key >> 4) & 7) as usize + 16;
        if key & 1 != 0 && pos + run < len {
            p[pos..pos + run].iter_mut().for_each(|b| *b ^= 0xff);
        }
        step += 1;
        key = key.wrapping_add(step);
        pos += run;
    }
    let records = u24(p, 4);
    for r in 0..records {
        let at = MZB_RECORDS_AT + r * MZB_RECORD;
        let Some(name) = p.get_mut(at..at + 16) else { break };
        name.iter_mut().for_each(|b| *b ^= 0x55);
    }
    Decoded::Yes
}

/// One object placement from a decoded MZB payload.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    /// Name of the MMB mesh this places.
    pub mesh: String,
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
}

fn f32_at(p: &[u8], i: usize) -> f32 {
    f32::from_le_bytes([p[i], p[i + 1], p[i + 2], p[i + 3]])
}

/// Reads placement records from a decoded MZB payload.
pub fn placements(p: &[u8]) -> Vec<Placement> {
    if p.len() < MZB_RECORDS_AT {
        return Vec::new();
    }
    (0..u24(p, 4))
        .map_while(|r| {
            let at = MZB_RECORDS_AT + r * MZB_RECORD;
            let rec = p.get(at..at + MZB_RECORD)?;
            let v = |k: usize| f32_at(rec, 16 + 4 * k);
            Some(Placement {
                mesh: String::from_utf8_lossy(&rec[..16]).trim_end_matches([' ', '\0']).to_string(),
                position: [v(0), v(1), v(2)],
                rotation: [v(3), v(4), v(5)],
                scale: [v(6), v(7), v(8)],
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> ZoneKeys {
        let mut a = [0u8; 256];
        let mut b = [0u8; 256];
        for i in 0..256 {
            a[i] = (i as u8).wrapping_mul(7).wrapping_add(3);
            b[i] = (i as u8).wrapping_mul(13).wrapping_add(1);
        }
        ZoneKeys::from_tables(a, b)
    }

    #[test]
    fn table_search_finds_fingerprinted_window() {
        let table: Vec<u8> = (0..=255u8).rev().collect();
        let fp = Fingerprint { sum: table.iter().map(|&b| b as u32).sum(), fnv: fnv1a(&table) };
        let mut image = vec![0x11u8; 1000];
        image[300..556].copy_from_slice(&table);
        assert_eq!(find_table(&image, &fp).unwrap().as_slice(), table.as_slice());
        assert!(find_table(&image[..400], &fp).is_none());
    }

    #[test]
    fn mmb_decode_is_an_involution_on_the_xor_pass() {
        // Version 5, swap pass off: decoding twice must restore the input.
        let mut p = vec![0u8; 64];
        p[0] = 64;
        p[3] = 5;
        p[5] = 0x42;
        for (i, b) in p[8..].iter_mut().enumerate() {
            *b = i as u8;
        }
        let orig = p.clone();
        assert_eq!(decode_mmb(&mut p, &keys()), Decoded::Yes);
        assert_ne!(p, orig);
        decode_mmb(&mut p, &keys());
        assert_eq!(p, orig);
    }

    #[test]
    fn mmb_swap_pass_exchanges_halves_when_key_bit_set() {
        let k = keys();
        let mut p = vec![0u8; 8 + 32];
        p[0] = 40;
        p[3] = 4; // no xor pass
        p[5] = 0xf0; // k1 = 0
        p[6] = 0xff;
        p[7] = 0xff;
        p[8..24].copy_from_slice(&[1; 16]);
        p[24..40].copy_from_slice(&[2; 16]);
        decode_mmb(&mut p, &k);
        // k2 starts at swap_keys[0] = 1 (odd) -> first 8-byte pair swapped.
        assert_eq!(&p[8..16], &[2; 8]);
        assert_eq!(&p[24..32], &[1; 8]);
    }

    #[test]
    fn mzb_names_and_placements() {
        let mut p = vec![0u8; MZB_RECORDS_AT + MZB_RECORD];
        p[0] = 8; // decode length 8: no inversion runs, isolates the name pass
        p[3] = 0x1b;
        p[4] = 1;
        for (i, c) in b"hut_a".iter().enumerate() {
            p[MZB_RECORDS_AT + i] = c ^ 0x55;
        }
        for i in 5..16 {
            p[MZB_RECORDS_AT + i] = 0x55;
        }
        p[MZB_RECORDS_AT + 16..MZB_RECORDS_AT + 20].copy_from_slice(&100f32.to_le_bytes());
        assert_eq!(decode_mzb(&mut p, &keys()), Decoded::Yes);
        let pl = placements(&p);
        assert_eq!(pl.len(), 1);
        assert_eq!(pl[0].mesh, "hut_a");
        assert_eq!(pl[0].position[0], 100.0);
    }

    #[test]
    fn magic_headers_are_plain() {
        let mut p = b"MMB   rest".to_vec();
        assert_eq!(decode_mmb(&mut p, &keys()), Decoded::AlreadyPlain);
        let mut z = vec![0u8; 64];
        z[..4].copy_from_slice(b"MZB");
        assert_eq!(decode_mzb(&mut z, &keys()), Decoded::AlreadyPlain);
    }

    #[test]
    fn bad_length_changes_nothing() {
        let mut p = vec![0u8; 16];
        p[0] = 0xff;
        p[3] = 5;
        let orig = p.clone();
        assert_eq!(decode_mmb(&mut p, &keys()), Decoded::BadLength);
        assert_eq!(p, orig);
    }
}
