//! Actor skeleton (chunk kind 0x29).
//!
//! Payload layout (little-endian):
//! - 0..2: unknown (version/flags), 2..4: u16 bone count, then `count` bones.
//! - bone (30 bytes): u8 parent index, u8 leaf flag, quaternion as 4 x f32
//!   (x, y, z, w), translation as 3 x f32. Bone 0 is the root and its parent
//!   byte is ignored. Rotations use the standard convention (v' = q v q*,
//!   local to the parent; world = parent * local). Trailing data after the bones (reference points) is not read.
//!
//! Parents always come before their children, so world transforms are built in one pass.

use std::fmt;

pub const KIND: u8 = 0x29;
const BONE_LEN: usize = 30;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bone {
    /// `None` for the root.
    pub parent: Option<usize>,
    pub leaf: bool,
    /// Local rotation (x, y, z, w).
    pub rotation: [f32; 4],
    /// Local translation relative to the parent.
    pub translation: [f32; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Skeleton {
    pub bones: Vec<Bone>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkeletonError {
    Truncated { need: usize, len: usize },
    /// Bone `bone` names a parent that is not an earlier bone.
    BadParent { bone: usize, parent: usize },
    Empty,
}

impl fmt::Display for SkeletonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { need, len } => write!(f, "skeleton needs {need} bytes, payload is {len}"),
            Self::BadParent { bone, parent } => write!(f, "bone {bone} has parent {parent} (not an earlier bone)"),
            Self::Empty => write!(f, "skeleton has no bones"),
        }
    }
}

impl std::error::Error for SkeletonError {}

pub fn parse(p: &[u8]) -> Result<Skeleton, SkeletonError> {
    if p.len() < 4 {
        return Err(SkeletonError::Truncated { need: 4, len: p.len() });
    }
    let count = u16::from_le_bytes([p[2], p[3]]) as usize;
    if count == 0 {
        return Err(SkeletonError::Empty);
    }
    let need = 4 + count * BONE_LEN;
    if p.len() < need {
        return Err(SkeletonError::Truncated { need, len: p.len() });
    }
    let f = |at: usize| f32::from_le_bytes([p[at], p[at + 1], p[at + 2], p[at + 3]]);
    let mut bones = Vec::with_capacity(count);
    for i in 0..count {
        let b = 4 + i * BONE_LEN;
        let parent = if i == 0 { None } else { Some(p[b] as usize) };
        if let Some(par) = parent {
            if par >= i {
                return Err(SkeletonError::BadParent { bone: i, parent: par });
            }
        }
        bones.push(Bone {
            parent,
            leaf: p[b + 1] != 0,
            rotation: [f(b + 2), f(b + 6), f(b + 10), f(b + 14)],
            translation: [f(b + 18), f(b + 22), f(b + 26)],
        });
    }
    Ok(Skeleton { bones })
}

/// Rotation (unit quaternion x,y,z,w) plus translation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub rotation: [f32; 4],
    pub translation: [f32; 3],
}

impl Pose {
    pub const IDENTITY: Pose = Pose { rotation: [0.0, 0.0, 0.0, 1.0], translation: [0.0; 3] };

    pub fn apply(&self, v: [f32; 3]) -> [f32; 3] {
        let r = quat_rotate(self.rotation, v);
        [r[0] + self.translation[0], r[1] + self.translation[1], r[2] + self.translation[2]]
    }

    /// `self` applied after `child`: world = parent.compose(local).
    pub fn compose(&self, child: &Pose) -> Pose {
        Pose { rotation: quat_mul(self.rotation, child.rotation), translation: self.apply(child.translation) }
    }
}

pub fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

pub fn quat_rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    // v' = v + 2w(u x v) + 2 u x (u x v), u = (x, y, z)
    let u = [q[0], q[1], q[2]];
    let c1 = cross(u, v);
    let c2 = cross(u, c1);
    [v[0] + 2.0 * (q[3] * c1[0] + c2[0]), v[1] + 2.0 * (q[3] * c1[1] + c2[1]), v[2] + 2.0 * (q[3] * c1[2] + c2[2])]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

pub fn quat_normalize(q: [f32; 4]) -> [f32; 4] {
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if n < 1e-12 { [0.0, 0.0, 0.0, 1.0] } else { [q[0] / n, q[1] / n, q[2] / n, q[3] / n] }
}

impl Skeleton {
    /// World pose of every bone from local poses (one per bone).
    pub fn world_from_local(&self, local: &[Pose]) -> Vec<Pose> {
        let mut world: Vec<Pose> = Vec::with_capacity(self.bones.len());
        for (i, b) in self.bones.iter().enumerate() {
            let l = local.get(i).copied().unwrap_or(Pose::IDENTITY);
            world.push(match b.parent {
                Some(p) => world[p].compose(&l),
                None => l,
            });
        }
        world
    }

    /// The stored local pose of every bone.
    pub fn bind_local(&self) -> Vec<Pose> {
        self.bones.iter().map(|b| Pose { rotation: b.rotation, translation: b.translation }).collect()
    }

    /// World bind pose.
    pub fn bind_world(&self) -> Vec<Pose> {
        self.world_from_local(&self.bind_local())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bone(parent: u8, q: [f32; 4], t: [f32; 3]) -> Vec<u8> {
        let mut v = vec![parent, 0];
        for x in q.iter().chain(t.iter()) {
            v.extend(x.to_le_bytes());
        }
        v
    }

    fn payload(bones: &[Vec<u8>]) -> Vec<u8> {
        let mut p = vec![1, 0];
        p.extend((bones.len() as u16).to_le_bytes());
        for b in bones {
            p.extend(b);
        }
        p.extend([0u8; 16]); // trailing data is ignored
        p
    }

    #[test]
    fn parses_and_composes_world_pose() {
        let h = std::f32::consts::FRAC_1_SQRT_2;
        // Root at (0,1,0); child rotated 90 degrees about z offset (1,0,0) in the root frame.
        let p = payload(&[
            bone(0, [0.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
            bone(0, [0.0, 0.0, h, h], [1.0, 0.0, 0.0]),
            bone(1, [0.0, 0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
        ]);
        let s = parse(&p).unwrap();
        assert_eq!(s.bones.len(), 3);
        assert_eq!(s.bones[0].parent, None);
        assert_eq!(s.bones[2].parent, Some(1));
        let w = s.bind_world();
        assert!((w[1].translation[0] - 1.0).abs() < 1e-5 && (w[1].translation[1] - 1.0).abs() < 1e-5);
        // Grandchild offset (1,0,0) rotated 90 degrees about z becomes (0,1,0).
        assert!((w[2].translation[0] - 1.0).abs() < 1e-5, "{:?}", w[2]);
        assert!((w[2].translation[1] - 2.0).abs() < 1e-5, "{:?}", w[2]);
    }

    #[test]
    fn rejects_forward_parent_and_truncation() {
        let p = payload(&[bone(0, [0.0, 0.0, 0.0, 1.0], [0.0; 3]), bone(5, [0.0, 0.0, 0.0, 1.0], [0.0; 3])]);
        assert_eq!(parse(&p), Err(SkeletonError::BadParent { bone: 1, parent: 5 }));
        let p = payload(&[bone(0, [0.0, 0.0, 0.0, 1.0], [0.0; 3])]);
        assert!(matches!(parse(&p[..20]), Err(SkeletonError::Truncated { .. })));
        assert_eq!(parse(&[0, 0, 0, 0]), Err(SkeletonError::Empty));
    }
}
