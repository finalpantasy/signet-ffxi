//! Zone mesh (MMB, chunk kind 0x2E) geometry, read from a de-obfuscated payload
//! (see `crypt::decode_mmb`).
//!
//! Payload layout:
//! - 0..16: head. Either the ASCII magic "MMB" plus a bit-field word at 3..7
//!   (topology bits 0..6: 0 = triangle lists), or a u24 length, then byte 4 =
//!   vertex layout (2 = 48-byte vertices with an extra vector, triangle lists),
//!   and an 8-byte group name at 8..16.
//! - 16..64: mesh header: 16-byte name (what MZB placements refer to), i32 piece
//!   count, bounding box as (min x, max x, min y, max y, min z, max z), u32
//!   offset of the first piece.
//! - piece offsets: when the first-piece offset is 0, up to 8 u32 offsets follow
//!   the header (or one piece right after it if the count is 0); otherwise u32
//!   offsets fill the gap between the header and the first piece.
//! - piece: i32 sub-mesh count, bounding box (6 x f32), i32 face count.
//! - sub-mesh: 16-byte texture name, u16 vertex count, u16 blend flags,
//!   vertices, u32 index count (low 16 bits used), u16 indices padded to 4 bytes.
//! - vertex (36 bytes): position, normal, BGRA color, u, v. The 48-byte form adds
//!   a vector after the position.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// Raw 32-bit color as stored (BGRA byte order).
    pub color: u32,
    pub uv: [f32; 2],
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubMesh {
    pub texture: String,
    pub blend: u16,
    pub vertices: Vec<Vertex>,
    /// Triangle list (strips are converted, degenerate triangles dropped).
    pub indices: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mesh {
    pub group: String,
    pub name: String,
    /// (min, max) per axis.
    pub bounds: [[f32; 2]; 3],
    pub submeshes: Vec<SubMesh>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MmbError {
    Truncated { at: usize, need: usize, len: usize },
    TooManySubMeshes(i32),
    IndexOutOfRange { index: u32, vertices: usize },
}

impl fmt::Display for MmbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { at, need, len } => write!(f, "needs {need} bytes at {at}, payload is {len}"),
            Self::TooManySubMeshes(n) => write!(f, "implausible sub-mesh count {n}"),
            Self::IndexOutOfRange { index, vertices } => write!(f, "index {index} >= vertex count {vertices}"),
        }
    }
}

impl std::error::Error for MmbError {}

struct Reader<'a> {
    p: &'a [u8],
}

impl Reader<'_> {
    fn bytes(&self, at: usize, n: usize) -> Result<&[u8], MmbError> {
        self.p.get(at..at + n).ok_or(MmbError::Truncated { at, need: n, len: self.p.len() })
    }
    fn u16(&self, at: usize) -> Result<u16, MmbError> {
        let b = self.bytes(at, 2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&self, at: usize) -> Result<u32, MmbError> {
        let b = self.bytes(at, 4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn f32(&self, at: usize) -> Result<f32, MmbError> {
        self.u32(at).map(f32::from_bits)
    }
    fn name(&self, at: usize, n: usize) -> Result<String, MmbError> {
        let b = self.bytes(at, n)?;
        let end = b.iter().position(|&c| c == 0).unwrap_or(n);
        Ok(String::from_utf8_lossy(&b[..end]).trim_end().to_string())
    }
    fn bounds(&self, at: usize) -> Result<[[f32; 2]; 3], MmbError> {
        let v = |k: usize| self.f32(at + 4 * k);
        Ok([[v(0)?, v(1)?], [v(2)?, v(3)?], [v(4)?, v(5)?]])
    }
}

fn strip_to_list(strip: &[u16], out: &mut Vec<u32>) {
    for i in 2..strip.len() {
        let (a, b, c) = (strip[i - 2], strip[i - 1], strip[i]);
        if a == b || b == c || a == c {
            continue;
        }
        if i % 2 == 0 {
            out.extend([a as u32, b as u32, c as u32]);
        } else {
            out.extend([b as u32, a as u32, c as u32]);
        }
    }
}

/// Parses a de-obfuscated MMB payload.
pub fn parse(p: &[u8]) -> Result<Mesh, MmbError> {
    let r = Reader { p };
    r.bytes(0, 64)?;
    let magic = p.starts_with(b"MMB");
    let (lists, stride, group) = if magic {
        (r.u32(3)? & 0x7f == 0, 36, String::new())
    } else {
        (p[4] == 2, if p[4] == 2 { 48 } else { 36 }, r.name(8, 8)?)
    };
    let name = r.name(16, 16)?;
    let pieces = r.u32(32)? as i32;
    let bounds = r.bounds(36)?;
    let first = r.u32(60)? as usize;

    let mut offsets = Vec::new();
    let mut at = 64;
    if first == 0 {
        if pieces != 0 {
            for k in 0..8 {
                let o = r.u32(64 + 4 * k)? as usize;
                if o != 0 {
                    offsets.push(o);
                }
            }
            at = 96;
        } else {
            offsets.push(64);
        }
    } else {
        offsets.push(first);
        let mut k = 64;
        while k + 4 <= first {
            let o = r.u32(k)? as usize;
            if o != 0 {
                offsets.push(o);
            }
            k += 4;
        }
        at = first;
    }

    let mut submeshes = Vec::new();
    let mut offsets = offsets.into_iter();
    for _ in 0..pieces.max(0) {
        if let Some(o) = offsets.next() {
            at = o;
        }
        let count = r.u32(at)? as i32;
        if !(0..=64).contains(&count) {
            return Err(MmbError::TooManySubMeshes(count));
        }
        at += 32;
        for _ in 0..count {
            let texture = r.name(at, 16)?;
            let nverts = r.u16(at + 16)? as usize;
            let blend = r.u16(at + 18)?;
            at += 20;
            r.bytes(at, nverts * stride)?;
            let mut vertices = Vec::with_capacity(nverts);
            for v in 0..nverts {
                let b = at + v * stride;
                let n = if stride == 48 { b + 24 } else { b + 12 };
                vertices.push(Vertex {
                    position: [r.f32(b)?, r.f32(b + 4)?, r.f32(b + 8)?],
                    normal: [r.f32(n)?, r.f32(n + 4)?, r.f32(n + 8)?],
                    color: r.u32(n + 12)?,
                    uv: [r.f32(n + 16)?, r.f32(n + 20)?],
                });
            }
            at += nverts * stride;
            let nidx = (r.u32(at)? & 0xffff) as usize;
            at += 4;
            let raw: Vec<u16> = (0..nidx).map(|k| r.u16(at + 2 * k)).collect::<Result<_, _>>()?;
            at += nidx * 2 + (nidx % 2) * 2;
            let mut indices = Vec::new();
            if lists {
                indices.extend(raw.chunks_exact(3).flatten().map(|&i| i as u32));
            } else {
                strip_to_list(&raw, &mut indices);
            }
            if let Some(&bad) = indices.iter().find(|&&i| i as usize >= nverts) {
                return Err(MmbError::IndexOutOfRange { index: bad, vertices: nverts });
            }
            submeshes.push(SubMesh { texture, blend, vertices, indices });
        }
    }
    Ok(Mesh { group, name, bounds, submeshes })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_f32s(p: &mut Vec<u8>, v: &[f32]) {
        v.iter().for_each(|x| p.extend_from_slice(&x.to_le_bytes()));
    }

    /// One piece, one sub-mesh, a 4-vertex strip.
    fn sample(vertex_kind: u8) -> Vec<u8> {
        let mut p = vec![0u8; 16];
        p[4] = vertex_kind;
        p[8..12].copy_from_slice(b"grp1");
        let mut name = [b' '; 16];
        name[..4].copy_from_slice(b"hut1");
        p.extend_from_slice(&name);
        p.extend_from_slice(&1u32.to_le_bytes());
        put_f32s(&mut p, &[0.0, 1.0, 0.0, 1.0, 0.0, 1.0]);
        p.extend_from_slice(&64u32.to_le_bytes()); // first piece right after header
        p.extend_from_slice(&1u32.to_le_bytes()); // sub-mesh count
        put_f32s(&mut p, &[0.0; 6]);
        p.extend_from_slice(&0u32.to_le_bytes());
        let mut tex = [0u8; 16];
        tex[..5].copy_from_slice(b"wood1");
        p.extend_from_slice(&tex);
        p.extend_from_slice(&4u16.to_le_bytes());
        p.extend_from_slice(&0u16.to_le_bytes());
        for i in 0..4 {
            put_f32s(&mut p, &[i as f32, 0.0, 0.0]);
            if vertex_kind == 2 {
                put_f32s(&mut p, &[0.0; 3]);
            }
            put_f32s(&mut p, &[0.0, 1.0, 0.0]);
            p.extend_from_slice(&0xff80_8080u32.to_le_bytes());
            put_f32s(&mut p, &[0.5, 0.5]);
        }
        let idx: &[u16] = if vertex_kind == 2 { &[0, 1, 2] } else { &[0, 1, 2, 3] };
        p.extend_from_slice(&(idx.len() as u32).to_le_bytes());
        idx.iter().for_each(|i| p.extend_from_slice(&i.to_le_bytes()));
        p
    }

    #[test]
    fn strip_mesh_parses_and_converts() {
        let m = parse(&sample(1)).unwrap();
        assert_eq!((m.group.as_str(), m.name.as_str()), ("grp1", "hut1"));
        let s = &m.submeshes[0];
        assert_eq!(s.texture, "wood1");
        assert_eq!(s.vertices[3].position, [3.0, 0.0, 0.0]);
        assert_eq!(s.indices, vec![0, 1, 2, 2, 1, 3]);
    }

    #[test]
    fn wide_vertices_use_lists() {
        let m = parse(&sample(2)).unwrap();
        let s = &m.submeshes[0];
        assert_eq!(s.vertices[2].normal, [0.0, 1.0, 0.0]);
        assert_eq!(s.indices, vec![0, 1, 2]);
    }

    #[test]
    fn truncation_is_reported() {
        let p = sample(1);
        assert!(matches!(parse(&p[..p.len() - 4]), Err(MmbError::Truncated { .. })));
    }
}
