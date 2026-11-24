//! Actor mesh (chunk kind 0x2A): skinned vertices plus textured triangle runs.
//!
//! Payload layout (little-endian). All offsets in the header are in 2-byte units
//! from the start of the payload and are stored as the low u16 of a 4-byte field:
//! - 0: version, 1: unknown, 2..4: u16 type (low 7 bits: 0 = model, 1 = cloth;
//!   bit 7: bone indexes go through the bone table), 4..6: u16 flip flag
//!   (1 = the mesh also has a mirrored twin for the opposite side).
//! - 0x06: poly offset, 0x0C: bone-table offset (count u16 at 0x10),
//!   0x12: weight-count offset, 0x18: bone-list offset, 0x1E: vertex offset.
//! - weight counts: two u16, `single` then `dual`.
//! - vertices: `single` records of 24 bytes (position, normal; both local to
//!   one bone), then `dual` records of 56 bytes holding two influences,
//!   interleaved per axis: x1 x2 y1 y2 z1 z2 w1 w2, then normal x1 x2 y1 y2 z1 z2.
//!   Cloth meshes use 12-byte (position) and 32-byte (x1 x2 y1 y2 z1 z2 w1 w2)
//!   records and carry no normals.
//! - bone list: 4 bytes per vertex (single then dual), two u16: first influence,
//!   second influence (unused for singles). Each u16: bits 0..7 bone index for
//!   the normal mesh, bits 7..14 bone index for the mirrored twin, bits 14..16
//!   mirror axis for the twin (0 none, 1 x, 2 y, 3 z).
//! - polygon stream at the poly offset, 4-byte records `u16 tag, u16 count`:
//!   tag & 0x80F0 == 0x8000: 16-byte texture name follows (18 bytes total);
//!   tag & 0x80F0 == 0x8010: 46 bytes, skipped; "T" (0x0054): `count` triangles
//!   of 30 bytes (3 x i16 vertex index, 3 x (u, v) f32); "ST" (0x5453): a strip
//!   whose first triangle is 30 bytes and then `count - 1` further vertices of
//!   10 bytes (i16 index, u, v); "SC" (0x4353) and "C" (0x0043) are colored runs
//!   that are skipped. Anything else ends the stream.

use std::fmt;

pub const KIND: u8 = 0x2A;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Influence {
    /// Bone index for the normal mesh.
    pub bone: u16,
    /// Bone index for the mirrored twin.
    pub bone_mirror: u16,
    /// Mirror axis applied for the twin (0 none, 1 x, 2 y, 3 z).
    pub mirror_axis: u8,
    pub weight: f32,
    /// Position local to the bone (already scaled by the weight for dual vertices).
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkinVertex {
    /// One or two influences.
    pub influences: Vec<Influence>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Corner {
    pub vertex: u32,
    pub uv: [f32; 2],
}

/// Triangles that share a texture.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub texture: String,
    /// Triangle list, three corners per triangle (strips converted, degenerates dropped).
    pub corners: Vec<Corner>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActorMesh {
    pub cloth: bool,
    pub flip: bool,
    pub vertices: Vec<SkinVertex>,
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeshError {
    Truncated { at: usize, need: usize, len: usize },
    VertexIndexOutOfRange { index: u32, vertices: usize },
}

impl fmt::Display for MeshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { at, need, len } => write!(f, "needs {need} bytes at {at}, payload is {len}"),
            Self::VertexIndexOutOfRange { index, vertices } => write!(f, "vertex index {index} >= {vertices}"),
        }
    }
}

impl std::error::Error for MeshError {}

struct Reader<'a> {
    p: &'a [u8],
}

impl Reader<'_> {
    fn bytes(&self, at: usize, n: usize) -> Result<&[u8], MeshError> {
        self.p.get(at..at.checked_add(n).unwrap_or(usize::MAX)).ok_or(MeshError::Truncated {
            at,
            need: n,
            len: self.p.len(),
        })
    }
    fn u16(&self, at: usize) -> Result<u16, MeshError> {
        let b = self.bytes(at, 2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn i16(&self, at: usize) -> Result<i16, MeshError> {
        self.u16(at).map(|v| v as i16)
    }
    fn f32(&self, at: usize) -> Result<f32, MeshError> {
        let b = self.bytes(at, 4)?;
        Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn v3(&self, at: usize) -> Result<[f32; 3], MeshError> {
        Ok([self.f32(at)?, self.f32(at + 4)?, self.f32(at + 8)?])
    }
}

fn strip_to_list(strip: &[Corner], out: &mut Vec<Corner>) {
    for i in 2..strip.len() {
        let (a, b, c) = (strip[i - 2], strip[i - 1], strip[i]);
        if a.vertex == b.vertex || b.vertex == c.vertex || a.vertex == c.vertex {
            continue;
        }
        if i % 2 == 0 {
            out.extend([a, b, c]);
        } else {
            out.extend([b, a, c]);
        }
    }
}

pub fn parse(p: &[u8]) -> Result<ActorMesh, MeshError> {
    let r = Reader { p };
    r.bytes(0, 0x24)?;
    let ty = r.u16(2)?;
    let cloth = ty & 0x7f == 1;
    let indirect = ty & 0x80 != 0;
    let flip = r.u16(4)? != 0;
    let poly_at = r.u16(6)? as usize * 2;
    let tbl_at = r.u16(0x0C)? as usize * 2;
    let tbl_n = r.u16(0x10)? as usize;
    let weight_at = r.u16(0x12)? as usize * 2;
    let bone_at = r.u16(0x18)? as usize * 2;
    let vert_at = r.u16(0x1E)? as usize * 2;

    let single = r.u16(weight_at)? as usize;
    let dual = r.u16(weight_at + 2)? as usize;
    let table: Vec<u16> = (0..tbl_n).map(|i| r.u16(tbl_at + 2 * i)).collect::<Result<_, _>>()?;
    let resolve = |idx: u16| -> u16 { if indirect { table.get(idx as usize).copied().unwrap_or(0) } else { idx } };
    let decode = |w: u16| (resolve(w & 0x7f), resolve((w >> 7) & 0x7f), (w >> 14) as u8);

    let (single_len, dual_len) = if cloth { (12, 32) } else { (24, 56) };
    r.bytes(vert_at, single * single_len + dual * dual_len)?;
    r.bytes(bone_at, (single + dual) * 4)?;

    let mut vertices = Vec::with_capacity(single + dual);
    for i in 0..single {
        let at = vert_at + i * single_len;
        let (bone, bone_mirror, mirror_axis) = decode(r.u16(bone_at + i * 4)?);
        vertices.push(SkinVertex {
            influences: vec![Influence {
                bone,
                bone_mirror,
                mirror_axis,
                weight: 1.0,
                position: r.v3(at)?,
                normal: if cloth { [0.0; 3] } else { r.v3(at + 12)? },
            }],
        });
    }
    let dual_at = vert_at + single * single_len;
    for i in 0..dual {
        let at = dual_at + i * dual_len;
        let f = |k: usize| r.f32(at + 4 * k);
        let bo = bone_at + (single + i) * 4;
        let (b1, m1, a1) = decode(r.u16(bo)?);
        let (b2, m2, a2) = decode(r.u16(bo + 2)?);
        let (n1, n2) = if cloth {
            ([0.0; 3], [0.0; 3])
        } else {
            ([f(8)?, f(10)?, f(12)?], [f(9)?, f(11)?, f(13)?])
        };
        vertices.push(SkinVertex {
            influences: vec![
                Influence {
                    bone: b1,
                    bone_mirror: m1,
                    mirror_axis: a1,
                    weight: f(6)?,
                    position: [f(0)?, f(2)?, f(4)?],
                    normal: n1,
                },
                Influence {
                    bone: b2,
                    bone_mirror: m2,
                    mirror_axis: a2,
                    weight: f(7)?,
                    position: [f(1)?, f(3)?, f(5)?],
                    normal: n2,
                },
            ],
        });
    }

    let mut groups: Vec<Group> = Vec::new();
    let mut texture = String::new();
    let mut pos = poly_at;
    let corner = |at: usize, uv_at: usize| -> Result<Corner, MeshError> {
        Ok(Corner { vertex: r.i16(at)? as u16 as u32, uv: [r.f32(uv_at)?, r.f32(uv_at + 4)?] })
    };
    let mut push = |texture: &str, tris: Vec<Corner>| {
        if tris.is_empty() {
            return;
        }
        match groups.last_mut() {
            Some(g) if g.texture == texture => g.corners.extend(tris),
            _ => groups.push(Group { texture: texture.to_string(), corners: tris }),
        }
    };
    while pos + 4 <= p.len() {
        let tag = r.u16(pos)?;
        let n = r.u16(pos + 2)? as usize;
        if tag & 0x80F0 == 0x8010 {
            pos += 0x2E;
        } else if tag & 0x80F0 == 0x8000 {
            let b = r.bytes(pos + 2, 16)?;
            let end = b.iter().position(|&c| c == 0).unwrap_or(16);
            texture = String::from_utf8_lossy(&b[..end]).trim_end().to_string();
            pos += 0x12;
        } else if tag == 0x0054 {
            pos += 4;
            let mut tris = Vec::with_capacity(n * 3);
            for _ in 0..n {
                let c = [corner(pos, pos + 6)?, corner(pos + 2, pos + 14)?, corner(pos + 4, pos + 22)?];
                if c[0].vertex != c[1].vertex && c[1].vertex != c[2].vertex && c[0].vertex != c[2].vertex {
                    tris.extend(c);
                }
                pos += 30;
            }
            push(&texture, tris);
        } else if tag == 0x5453 {
            pos += 4;
            let mut strip = vec![corner(pos, pos + 6)?, corner(pos + 2, pos + 14)?, corner(pos + 4, pos + 22)?];
            pos += 30;
            for _ in 1..n {
                strip.push(corner(pos, pos + 2)?);
                pos += 10;
            }
            let mut tris = Vec::new();
            strip_to_list(&strip, &mut tris);
            push(&texture, tris);
        } else if tag == 0x4353 {
            pos += n * 20 + 0x0C;
        } else if tag == 0x0043 {
            pos += n * 10 + 4;
        } else {
            break;
        }
    }
    for g in &groups {
        if let Some(c) = g.corners.iter().find(|c| c.vertex as usize >= vertices.len()) {
            return Err(MeshError::VertexIndexOutOfRange { index: c.vertex, vertices: vertices.len() });
        }
    }
    Ok(ActorMesh { cloth, flip, vertices, groups })
}

impl ActorMesh {
    /// Skins every vertex with world bone poses. `mirrored` selects the mirrored
    /// twin (bone indexes and mirror axes for the opposite side). Returns
    /// (position, normal) per vertex; bones outside `world` contribute the
    /// untransformed local position.
    pub fn skin(&self, world: &[crate::skeleton::Pose], mirrored: bool) -> Vec<([f32; 3], [f32; 3])> {
        use crate::skeleton::quat_rotate;
        self.vertices
            .iter()
            .map(|v| {
                let mut pos = [0.0f32; 3];
                let mut nrm = [0.0f32; 3];
                for inf in &v.influences {
                    let (bone, axis) = if mirrored { (inf.bone_mirror, inf.mirror_axis) } else { (inf.bone, 0) };
                    let mut lp = inf.position;
                    let mut ln = inf.normal;
                    if axis > 0 {
                        lp[axis as usize - 1] = -lp[axis as usize - 1];
                        ln[axis as usize - 1] = -ln[axis as usize - 1];
                    }
                    let (rp, rn, t) = match world.get(bone as usize) {
                        Some(w) => (quat_rotate(w.rotation, lp), quat_rotate(w.rotation, ln), w.translation),
                        None => (lp, ln, [0.0; 3]),
                    };
                    for k in 0..3 {
                        pos[k] += rp[k] + t[k] * inf.weight;
                        nrm[k] += rn[k];
                    }
                }
                let l = (nrm[0] * nrm[0] + nrm[1] * nrm[1] + nrm[2] * nrm[2]).sqrt();
                if l > 1e-6 {
                    nrm = [nrm[0] / l, nrm[1] / l, nrm[2] / l];
                }
                (pos, nrm)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le16(v: &mut Vec<u8>, x: u16) {
        v.extend(x.to_le_bytes());
    }
    fn lef(v: &mut Vec<u8>, x: f32) {
        v.extend(x.to_le_bytes());
    }
    fn set16(p: &mut [u8], at: usize, x: u16) {
        p[at..at + 2].copy_from_slice(&x.to_le_bytes());
    }

    /// 1 single vertex (bone 3), 1 dual vertex (bones 1 and 2), one triangle
    /// list triangle and one 4-vertex strip with a degenerate triangle.
    fn sample() -> Vec<u8> {
        let mut p = vec![0u8; 0x40];
        set16(&mut p, 2, 0); // model
        let w_at = p.len();
        le16(&mut p, 2);
        le16(&mut p, 1);
        let b_at = p.len();
        le16(&mut p, 3);
        le16(&mut p, 0);
        le16(&mut p, 3);
        le16(&mut p, 0);
        le16(&mut p, 1 | (2 << 7) | (1 << 14));
        le16(&mut p, 2);
        let v_at = p.len();
        for x in [1.0, 2.0, 3.0, 0.0, 1.0, 0.0, 9.0, 9.0, 9.0, 0.0, 1.0, 0.0] {
            lef(&mut p, x);
        }
        // dual: x1 x2 y1 y2 z1 z2 w1 w2 nx1 nx2 ny1 ny2 nz1 nz2
        for x in [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 0.25, 0.75, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0] {
            lef(&mut p, x);
        }
        let poly = p.len();
        le16(&mut p, 0x8000);
        p.extend(b"skin_tex\0\0\0\0\0\0\0\0");
        le16(&mut p, 0x0054);
        le16(&mut p, 1);
        for i in [0u16, 1, 2] {
            le16(&mut p, i);
        }
        for uv in [0.0f32, 0.0, 1.0, 0.0, 0.0, 1.0] {
            lef(&mut p, uv);
        }
        le16(&mut p, 0x5453);
        le16(&mut p, 2);
        for i in [0u16, 1, 2] {
            le16(&mut p, i);
        }
        for uv in [0.0f32, 0.0, 1.0, 0.0, 0.0, 1.0] {
            lef(&mut p, uv);
        }
        le16(&mut p, 0);
        lef(&mut p, 0.5);
        lef(&mut p, 0.5);
        le16(&mut p, 0x9999); // end of stream
        le16(&mut p, 0);
        set16(&mut p, 6, (poly / 2) as u16);
        set16(&mut p, 0x12, (w_at / 2) as u16);
        set16(&mut p, 0x18, (b_at / 2) as u16);
        set16(&mut p, 0x1E, (v_at / 2) as u16);
        p
    }

    #[test]
    fn parses_vertices_bones_and_polygons() {
        let m = parse(&sample()).unwrap();
        assert_eq!(m.vertices.len(), 3);
        let s = &m.vertices[0].influences;
        assert_eq!((s.len(), s[0].bone, s[0].position, s[0].normal), (1, 3, [1.0, 2.0, 3.0], [0.0, 1.0, 0.0]));
        let d = &m.vertices[2].influences;
        assert_eq!((d[0].bone, d[0].bone_mirror, d[0].mirror_axis, d[0].weight), (1, 2, 1, 0.25));
        assert_eq!((d[1].bone, d[1].position, d[1].weight), (2, [2.0, 4.0, 6.0], 0.75));
        assert_eq!(m.groups.len(), 1);
        assert_eq!(m.groups[0].texture, "skin_tex");
        // One list triangle (0,1,2) plus strip (0,1,2,0): triangles (0,1,2) and (1,0,2)... 
        // the second strip triangle is (1,2,0) in odd order = (2,1,0).
        let ids: Vec<u32> = m.groups[0].corners.iter().map(|c| c.vertex).collect();
        assert_eq!(ids, vec![0, 1, 2, 0, 1, 2, 2, 1, 0]);
        assert_eq!(m.groups[0].corners[1].uv, [1.0, 0.0]);
    }

    #[test]
    fn skin_adds_weighted_translation() {
        use crate::skeleton::Pose;
        let m = parse(&sample()).unwrap();
        let mut world = vec![Pose::IDENTITY; 4];
        world[1].translation = [4.0, 0.0, 0.0];
        world[2].translation = [0.0, 4.0, 0.0];
        world[3].translation = [0.0, 0.0, 10.0];
        let s = m.skin(&world, false);
        assert_eq!(s[0].0, [1.0, 2.0, 13.0]);
        // dual: p1 + p2 + w1 * T1 + w2 * T2
        assert_eq!(s[2].0, [1.0 + 2.0 + 1.0, 3.0 + 4.0 + 3.0, 5.0 + 6.0]);
    }

    #[test]
    fn truncated_is_an_error() {
        let p = sample();
        assert!(matches!(parse(&p[..0x20]), Err(MeshError::Truncated { .. })));
        assert!(matches!(parse(&p[..0x50]), Err(MeshError::Truncated { .. })));
    }

    #[test]
    fn strip_conversion_drops_degenerates_and_alternates_winding() {
        let c = |v: u32| Corner { vertex: v, uv: [0.0; 2] };
        let strip = [c(0), c(1), c(2), c(3), c(3), c(4)];
        let mut out = Vec::new();
        strip_to_list(&strip, &mut out);
        let ids: Vec<u32> = out.iter().map(|c| c.vertex).collect();
        assert_eq!(ids, vec![0, 1, 2, 2, 1, 3]);
    }
}
