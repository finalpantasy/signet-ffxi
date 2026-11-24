//! Actor animation (chunk kind 0x2B).
//!
//! Payload layout (little-endian):
//! - 0: version, 1: unknown, 2..4: u16 track count, 4..6: u16 frame count,
//!   6..10: f32 playback speed (frames advance by `speed` per game tick).
//! - 10..: a float pool. It starts with `count` track records of 84 bytes
//!   (21 words), then the key data. Channel indexes are word offsets into the
//!   pool, so an index of `n` means the u32/f32 at byte `10 + 4n`.
//! - track record words: bone index; then rotation x, y, z, w indexes; rotation
//!   x, y, z, w constants; translation x, y, z indexes; translation x, y, z
//!   constants; scale x, y, z indexes; scale x, y, z constants.
//!   An index of 0 means the channel is the constant; otherwise the channel reads
//!   `frames` consecutive floats at that pool index.
//!   Bit 31 on a rotation index means the track leaves the bone at its bind pose.
//! - Values are deltas on the bind pose: the sampled rotation is multiplied in
//!   front of the bind rotation (`rot = anim * bind`) and the sampled translation is
//!   added to the bind translation. Checked against the data: with this rule the
//!   feet of a walk cycle stay on the ground in every frame.

use std::fmt;

use crate::skeleton::{Pose, quat_mul, quat_normalize};

pub const KIND: u8 = 0x2B;
const HEAD: usize = 10;
const TRACK_WORDS: usize = 21;

#[derive(Debug, Clone, PartialEq)]
pub enum Channel {
    Const(f32),
    /// One value per frame.
    Keys(Vec<f32>),
}

impl Channel {
    fn at(&self, frame: f32) -> f32 {
        match self {
            Channel::Const(v) => *v,
            Channel::Keys(k) => {
                let last = k.len() - 1;
                let f = frame.clamp(0.0, last as f32);
                let i = f.floor() as usize;
                let j = (i + 1).min(last);
                let t = f - i as f32;
                k[i] * (1.0 - t) + k[j] * t
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub bone: usize,
    /// Bind pose is kept for this bone.
    pub skip: bool,
    pub rotation: [Channel; 4],
    pub translation: [Channel; 3],
    pub scale: [Channel; 3],
}

impl Track {
    /// Sampled local rotation (unit quaternion), translation and scale at a frame.
    pub fn sample(&self, frame: f32) -> ([f32; 4], [f32; 3], [f32; 3]) {
        let r = [
            self.rotation[0].at(frame),
            self.rotation[1].at(frame),
            self.rotation[2].at(frame),
            self.rotation[3].at(frame),
        ];
        let t = [self.translation[0].at(frame), self.translation[1].at(frame), self.translation[2].at(frame)];
        let s = [self.scale[0].at(frame), self.scale[1].at(frame), self.scale[2].at(frame)];
        (quat_normalize(r), t, s)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Animation {
    pub frames: u16,
    pub speed: f32,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnimError {
    Truncated { need: usize, len: usize },
    /// A channel's keys fall outside the payload.
    KeysOutOfRange { track: usize, index: u32 },
    NoFrames,
}

impl fmt::Display for AnimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { need, len } => write!(f, "animation needs {need} bytes, payload is {len}"),
            Self::KeysOutOfRange { track, index } => write!(f, "track {track} key index {index} outside payload"),
            Self::NoFrames => write!(f, "animation has zero frames"),
        }
    }
}

impl std::error::Error for AnimError {}

pub fn parse(p: &[u8]) -> Result<Animation, AnimError> {
    if p.len() < HEAD {
        return Err(AnimError::Truncated { need: HEAD, len: p.len() });
    }
    let count = u16::from_le_bytes([p[2], p[3]]) as usize;
    let frames = u16::from_le_bytes([p[4], p[5]]);
    let speed = f32::from_le_bytes([p[6], p[7], p[8], p[9]]);
    if frames == 0 {
        return Err(AnimError::NoFrames);
    }
    let need = HEAD + count * TRACK_WORDS * 4;
    if p.len() < need {
        return Err(AnimError::Truncated { need, len: p.len() });
    }
    let word = |w: usize| -> u32 {
        let at = HEAD + w * 4;
        u32::from_le_bytes([p[at], p[at + 1], p[at + 2], p[at + 3]])
    };
    let pool_words = (p.len() - HEAD) / 4;
    let mut tracks = Vec::with_capacity(count);
    for t in 0..count {
        let base = t * TRACK_WORDS;
        let chan = |idx_word: usize, const_word: usize, mask: bool| -> Result<Channel, AnimError> {
            let mut idx = word(base + idx_word);
            if mask {
                idx &= 0x7fff_ffff;
            }
            if idx == 0 {
                return Ok(Channel::Const(f32::from_bits(word(base + const_word))));
            }
            let end = idx as usize + frames as usize;
            if end > pool_words {
                return Err(AnimError::KeysOutOfRange { track: t, index: idx });
            }
            Ok(Channel::Keys((0..frames as usize).map(|f| f32::from_bits(word(idx as usize + f))).collect()))
        };
        let skip = (1..=4).any(|k| word(base + k) & 0x8000_0000 != 0);
        let rotation = [chan(1, 5, true)?, chan(2, 6, true)?, chan(3, 7, true)?, chan(4, 8, true)?];
        let translation = [chan(9, 12, false)?, chan(10, 13, false)?, chan(11, 14, false)?];
        let scale = [chan(15, 18, false)?, chan(16, 19, false)?, chan(17, 20, false)?];
        tracks.push(Track { bone: word(base) as usize, skip, rotation, translation, scale });
    }
    Ok(Animation { frames, speed, tracks })
}

impl Animation {
    /// Local poses for a skeleton at a (fractional) frame: tracks override the
    /// bind pose for the bones they name; other bones keep their bind pose.
    pub fn local_poses(&self, bind_local: &[Pose], frame: f32) -> Vec<Pose> {
        let mut out = bind_local.to_vec();
        for t in &self.tracks {
            if t.skip || t.bone >= out.len() {
                continue;
            }
            let (r, tr, _s) = t.sample(frame);
            let bind = bind_local[t.bone];
            out[t.bone] = Pose {
                rotation: quat_mul(r, bind.rotation),
                translation: [bind.translation[0] + tr[0], bind.translation[1] + tr[1], bind.translation[2] + tr[2]],
            };
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_constant_and_keyed_channels() {
        let frames = 3u16;
        let mut p = vec![1u8, 0];
        p.extend(1u16.to_le_bytes()); // one track
        p.extend(frames.to_le_bytes());
        p.extend(0.5f32.to_le_bytes());
        // pool: track record (21 words) then 3 key floats at word 21
        let mut w = vec![0u32; TRACK_WORDS];
        w[0] = 7; // bone
        w[8] = 1.0f32.to_bits(); // qw const
        w[9] = 21; // tx keyed at word 21
        w[18] = 1.0f32.to_bits();
        w[19] = 1.0f32.to_bits();
        w[20] = 1.0f32.to_bits();
        for v in w {
            p.extend(v.to_le_bytes());
        }
        for v in [0.0f32, 2.0, 4.0] {
            p.extend(v.to_le_bytes());
        }
        let a = parse(&p).unwrap();
        assert_eq!((a.frames, a.speed, a.tracks.len()), (3, 0.5, 1));
        let t = &a.tracks[0];
        assert_eq!(t.bone, 7);
        let (q, tr, s) = t.sample(1.5);
        assert_eq!(q, [0.0, 0.0, 0.0, 1.0]);
        assert!((tr[0] - 3.0).abs() < 1e-6);
        assert_eq!(s, [1.0; 3]);
        // frames past the end clamp
        assert!((t.sample(9.0).1[0] - 4.0).abs() < 1e-6);
    }

    #[test]
    fn poses_apply_deltas_to_bind() {
        let mut p = vec![1u8, 0, 1, 0, 1, 0];
        p.extend(1.0f32.to_le_bytes());
        let mut w = vec![0u32; TRACK_WORDS];
        w[0] = 1; // bone 1
        let h = std::f32::consts::FRAC_1_SQRT_2;
        w[7] = h.to_bits(); // qz
        w[8] = h.to_bits(); // qw: 90 degrees about z
        w[12] = 2.0f32.to_bits(); // tx
        for v in w {
            p.extend(v.to_le_bytes());
        }
        let a = parse(&p).unwrap();
        let bind = vec![Pose::IDENTITY, Pose { rotation: [0.0, 0.0, 0.0, 1.0], translation: [1.0, 5.0, 0.0] }];
        let posed = a.local_poses(&bind, 0.0);
        assert_eq!(posed[0], Pose::IDENTITY);
        assert_eq!(posed[1].translation, [3.0, 5.0, 0.0]);
        assert!((posed[1].rotation[2] - h).abs() < 1e-6 && (posed[1].rotation[3] - h).abs() < 1e-6);
    }

    #[test]
    fn rejects_bad_key_index_and_truncation() {
        let mut p = vec![1u8, 0, 1, 0, 2, 0];
        p.extend(1.0f32.to_le_bytes());
        let mut w = vec![0u32; TRACK_WORDS];
        w[9] = 5000;
        for v in w {
            p.extend(v.to_le_bytes());
        }
        assert!(matches!(parse(&p), Err(AnimError::KeysOutOfRange { .. })));
        assert!(matches!(parse(&p[..30]), Err(AnimError::Truncated { .. })));
        assert!(matches!(parse(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 0]), Err(AnimError::NoFrames)));
    }
}
