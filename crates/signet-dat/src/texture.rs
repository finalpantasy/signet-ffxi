//! Texture chunks (kind 0x20).
//!
//! Payload layout, measured on a retail install (November 2026):
//! - byte 0: format flag
//! - bytes 1..17: label, two 8-byte ASCII names (group, texture)
//! - bytes 17..57: a 40-byte bitmap info header (width at +4, height at +8)
//! - flag 0xA1: 4-byte reversed FourCC ("1TXD" = DXT1, "3TXD", "5TXD"),
//!   u32 data size, u32 block-row pitch, then block data at byte 69.
//! - flag 0xB1: 4 unknown bytes, a 256-entry BGRA palette, then one index byte
//!   per pixel.
//! - flag 0x81: same as 0xB1 without the 4 unknown bytes (palette at byte 57).
//!   Seen on 48 effect and map textures; the data runs past width*height, so only
//!   the first width*height indices are used for now.
//!
//! Palette rows are stored bottom-up, like a Windows bitmap; decoding flips them so row 0
//! is the top. DXT block rows are stored top-down (verified 2026-11-23: a Hume face and the
//! West Ronfaure ground atlas are upside down if flipped, and the mesh UVs address the
//! stored rows directly, v = 0 is the first stored row). Palette alpha uses the PlayStation 2 range (0x80 = opaque) and is
//! doubled to 8-bit. DXT3 alpha is PS2 range too and gets the same
//! conversion (see [`ps2_alpha`]), so every consumer sees 8-bit alpha.

use std::fmt;

pub const KIND: u8 = 0x20;
const INFO: usize = 17;
const AFTER_INFO: usize = 57;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Dxt1,
    Dxt3,
    Dxt5,
    Palette8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureHeader {
    pub flag: u8,
    pub group: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub format: Format,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextureError {
    Truncated { need: usize, have: usize },
    UnknownFlag(u8),
    UnknownFourCc([u8; 4]),
    BadSize { width: i32, height: i32 },
}

impl fmt::Display for TextureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { need, have } => write!(f, "texture needs {need} bytes, has {have}"),
            Self::UnknownFlag(b) => write!(f, "unknown texture flag 0x{b:02x}"),
            Self::UnknownFourCc(c) => {
                write!(f, "unknown block format {:?}", String::from_utf8_lossy(c))
            }
            Self::BadSize { width, height } => write!(f, "bad texture size {width}x{height}"),
        }
    }
}

impl std::error::Error for TextureError {}

fn need(p: &[u8], n: usize) -> Result<(), TextureError> {
    if p.len() < n {
        Err(TextureError::Truncated { need: n, have: p.len() })
    } else {
        Ok(())
    }
}

fn label(b: &[u8]) -> String {
    String::from_utf8_lossy(b).trim_end_matches([' ', '\0']).to_string()
}

fn u32_at(p: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([p[i], p[i + 1], p[i + 2], p[i + 3]])
}

pub fn header(p: &[u8]) -> Result<TextureHeader, TextureError> {
    need(p, AFTER_INFO + 4)?;
    let width = u32_at(p, INFO + 4) as i32;
    let height = u32_at(p, INFO + 8) as i32;
    if !(1..=8192).contains(&width) || !(1..=8192).contains(&height) {
        return Err(TextureError::BadSize { width, height });
    }
    let format = match p[0] {
        0xa1 => match &p[AFTER_INFO..AFTER_INFO + 4] {
            b"1TXD" => Format::Dxt1,
            b"3TXD" => Format::Dxt3,
            b"5TXD" => Format::Dxt5,
            c => return Err(TextureError::UnknownFourCc([c[0], c[1], c[2], c[3]])),
        },
        0xb1 | 0x81 => Format::Palette8,
        f => return Err(TextureError::UnknownFlag(f)),
    };
    Ok(TextureHeader {
        flag: p[0],
        group: label(&p[1..9]),
        name: label(&p[9..17]),
        width: width as u32,
        height: height as u32,
        format,
    })
}

/// Decoded image, RGBA8, row 0 at the top.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn decode(p: &[u8]) -> Result<(TextureHeader, Image), TextureError> {
    let h = header(p)?;
    let (w, ht) = (h.width as usize, h.height as usize);
    let mut rgba = vec![0u8; w * ht * 4];
    match h.format {
        Format::Palette8 => {
            let pal = if h.flag == 0x81 { AFTER_INFO } else { AFTER_INFO + 4 };
            let px = pal + 1024;
            need(p, px + w * ht)?;
            for y in 0..ht {
                let src_row = ht - 1 - y;
                for x in 0..w {
                    let i = p[px + src_row * w + x] as usize * 4;
                    let o = (y * w + x) * 4;
                    let c = &p[pal + i..pal + i + 4];
                    rgba[o] = c[2];
                    rgba[o + 1] = c[1];
                    rgba[o + 2] = c[0];
                    rgba[o + 3] = if c[3] >= 0x80 { 255 } else { c[3] * 2 };
                }
            }
        }
        f => {
            let block_bytes = if f == Format::Dxt1 { 8 } else { 16 };
            let (bw, bh) = (w.div_ceil(4), ht.div_ceil(4));
            let data = AFTER_INFO + 12;
            need(p, data + bw * bh * block_bytes)?;
            for by in 0..bh {
                for bx in 0..bw {
                    let b = &p[data + (by * bw + bx) * block_bytes..][..block_bytes];
                    let texels = decode_block(f, b);
                    for ty in 0..4 {
                        for tx in 0..4 {
                            let (x, y) = (bx * 4 + tx, by * 4 + ty);
                            if x >= w || y >= ht {
                                continue;
                            }
                            rgba[(y * w + x) * 4..][..4].copy_from_slice(&texels[ty * 4 + tx]);
                        }
                    }
                }
            }
        }
    }
    Ok((h, Image { width: w as u32, height: ht as u32, rgba }))
}

/// PS2 alpha (0x80 = opaque) to 8-bit. DXT3 alpha is stored in this range too, quantized to 4
/// bits, so "opaque" arrives as 0x77 (nibble 7 * 17); values at or above 0xEC snap to 255.
pub fn ps2_alpha(a: u8) -> u8 {
    let d = (a as u32 * 2).min(255);
    if d >= 0xEC { 255 } else { d as u8 }
}

fn rgb565(c: u16) -> [u8; 3] {
    let r = (c >> 11) & 31;
    let g = (c >> 5) & 63;
    let b = c & 31;
    [(r * 255 / 31) as u8, (g * 255 / 63) as u8, (b * 255 / 31) as u8]
}

fn decode_block(f: Format, b: &[u8]) -> [[u8; 4]; 16] {
    let color = if f == Format::Dxt1 { b } else { &b[8..] };
    let c0 = u16::from_le_bytes([color[0], color[1]]);
    let c1 = u16::from_le_bytes([color[2], color[3]]);
    let (a, bb) = (rgb565(c0), rgb565(c1));
    let mix = |w0: u16, w1: u16, d: u16| -> [u8; 4] {
        let m = |i: usize| ((a[i] as u16 * w0 + bb[i] as u16 * w1) / d) as u8;
        [m(0), m(1), m(2), 255]
    };
    let palette: [[u8; 4]; 4] = if f != Format::Dxt1 || c0 > c1 {
        [[a[0], a[1], a[2], 255], [bb[0], bb[1], bb[2], 255], mix(2, 1, 3), mix(1, 2, 3)]
    } else {
        [[a[0], a[1], a[2], 255], [bb[0], bb[1], bb[2], 255], mix(1, 1, 2), [0, 0, 0, 0]]
    };
    let bits = u32::from_le_bytes([color[4], color[5], color[6], color[7]]);
    let mut out = [[0u8; 4]; 16];
    for (i, t) in out.iter_mut().enumerate() {
        *t = palette[((bits >> (2 * i)) & 3) as usize];
    }
    match f {
        Format::Dxt3 => {
            for (i, t) in out.iter_mut().enumerate() {
                let nib = (b[i / 2] >> (4 * (i % 2))) & 0xf;
                t[3] = ps2_alpha(nib * 17);
            }
        }
        Format::Dxt5 => {
            let (a0, a1) = (b[0] as u16, b[1] as u16);
            let mut al = [0u8; 8];
            al[0] = a0 as u8;
            al[1] = a1 as u8;
            if a0 > a1 {
                for k in 1..7 {
                    al[k + 1] = (((7 - k) as u16 * a0 + k as u16 * a1) / 7) as u8;
                }
            } else {
                for k in 1..5 {
                    al[k + 1] = (((5 - k) as u16 * a0 + k as u16 * a1) / 5) as u8;
                }
                al[6] = 0;
                al[7] = 255;
            }
            let mut idx = 0u64;
            for k in 0..6 {
                idx |= (b[2 + k] as u64) << (8 * k);
            }
            for (i, t) in out.iter_mut().enumerate() {
                t[3] = al[((idx >> (3 * i)) & 7) as usize];
            }
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tex(flag: u8, fourcc: &[u8; 4], w: u32, h: u32, body: &[u8]) -> Vec<u8> {
        let mut p = vec![flag];
        p.extend_from_slice(b"group   name    ");
        let mut info = [0u8; 40];
        info[0..4].copy_from_slice(&40u32.to_le_bytes());
        info[4..8].copy_from_slice(&w.to_le_bytes());
        info[8..12].copy_from_slice(&h.to_le_bytes());
        p.extend_from_slice(&info);
        p.extend_from_slice(fourcc);
        p.extend_from_slice(&[0; 8]);
        p.extend_from_slice(body);
        p
    }

    #[test]
    fn dxt1_solid_red() {
        let block = [0x00, 0xf8, 0x00, 0xf8, 0, 0, 0, 0];
        let (h, img) = decode(&tex(0xa1, b"1TXD", 4, 4, &block)).unwrap();
        assert_eq!(h.format, Format::Dxt1);
        assert_eq!(h.group, "group");
        assert_eq!(&img.rgba[..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn dxt3_alpha_nibbles_and_top_down_rows() {
        let mut block = [0u8; 16];
        block[0] = 0x0f; // texel 0 alpha 15 -> 255, texel 1 alpha 0
        block[8..12].copy_from_slice(&[0xff, 0xff, 0xff, 0xff]);
        let (_, img) = decode(&tex(0xa1, b"3TXD", 4, 4, &block)).unwrap();
        // Block row 0 is the top row of the image (DXT is not flipped).
        assert_eq!(img.rgba[3], 255);
        assert_eq!(img.rgba[7], 0);
    }

    #[test]
    fn dxt3_alpha_is_ps2_range() {
        assert_eq!((ps2_alpha(0), ps2_alpha(0x33), ps2_alpha(0x77), ps2_alpha(0x88), ps2_alpha(255)), (0, 0x66, 255, 255, 255));
        let mut block = [0u8; 16];
        block[0] = 0x73; // texel 0 nibble 3 -> 0x33 -> 0x66; texel 1 nibble 7 -> 0x77 -> opaque
        let (_, img) = decode(&tex(0xa1, b"3TXD", 4, 4, &block)).unwrap();
        assert_eq!((img.rgba[3], img.rgba[7]), (0x66, 255));
    }

    #[test]
    fn palette_0x81_has_no_gap_before_palette() {
        // tex() appends a 4-byte fourcc and 8 zero bytes; for 0x81 those 12 bytes are
        // the first 3 palette entries, so build the palette starting at byte 57.
        let mut p = tex(0x81, &[0, 0, 0xff, 0x80], 1, 1, &[]);
        p.truncate(57);
        let mut pal = [0u8; 1024];
        pal[4..8].copy_from_slice(&[0x00, 0x00, 0xff, 0x80]); // entry 1: BGRA red, PS2 opaque
        p.extend_from_slice(&pal);
        p.push(1);
        let (h, img) = decode(&p).unwrap();
        assert_eq!(h.format, Format::Palette8);
        assert_eq!(img.rgba, vec![255, 0, 0, 255]);
    }

    #[test]
    fn unknown_flag_is_reported() {
        assert_eq!(
            header(&tex(0x42, b"1TXD", 4, 4, &[0; 8])),
            Err(TextureError::UnknownFlag(0x42))
        );
    }
}
