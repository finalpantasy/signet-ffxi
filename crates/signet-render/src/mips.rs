//! Mip chains for zone textures, generated at upload.
//!
//! Three texture classes (see `classify`):
//! - `Opaque`: alpha is ignored when drawing; plain box filter.
//! - `Cutout`: drawn with an alpha test at `CUTOUT_REF`. Before downsampling, the RGB of
//!   fully transparent texels is filled from the nearest visible texel (alpha untouched) so
//!   dark fringes do not bleed in, and each level's alpha is scaled so the fraction of texels
//!   passing the test matches level 0, so thin leaves and grass do not thin out at distance.
//! - `Translucent`: alpha-blended; plain box filter.

/// Alpha-test reference (8-bit): texels with alpha at or above it are kept.
pub const CUTOUT_REF: u8 = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TexClass {
    Opaque,
    Cutout,
    Translucent,
}

pub struct Level {
    pub rgba: Vec<u8>,
    pub w: u32,
    pub h: u32,
}

/// Decides how a texture's alpha is used. Alpha must already be in the 0..255 range
/// (PS2 0x80 doubled).
pub fn classify(rgba: &[u8]) -> TexClass {
    let n = (rgba.len() / 4).max(1) as f64;
    let (mut clear, mut mid) = (0u64, 0u64);
    for p in rgba.chunks_exact(4) {
        if p[3] < CUTOUT_REF {
            clear += 1;
        }
        if (16..240).contains(&p[3]) {
            mid += 1;
        }
    }
    if mid as f64 / n >= 0.5 {
        TexClass::Translucent
    } else if clear as f64 / n >= 0.005 {
        TexClass::Cutout
    } else {
        TexClass::Opaque
    }
}

/// Mip chain for `class`; level 0 keeps the input alpha byte for byte.
pub fn chain(rgba: &[u8], w: u32, h: u32, class: TexClass) -> Vec<Level> {
    match class {
        TexClass::Cutout => cutout_chain(rgba, w, h),
        _ => box_chain(rgba, w, h),
    }
}

fn halve(src: &[u8], sw: u32, sh: u32) -> Level {
    let (dw, dh) = ((sw / 2).max(1), (sh / 2).max(1));
    let mut dst = vec![0u8; (dw * dh * 4) as usize];
    for y in 0..dh {
        for x in 0..dw {
            let mut acc = [0u32; 4];
            for (ox, oy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (sx, sy) = ((x * 2 + ox).min(sw - 1), (y * 2 + oy).min(sh - 1));
                let p = &src[((sy * sw + sx) * 4) as usize..][..4];
                for k in 0..4 {
                    acc[k] += p[k] as u32;
                }
            }
            for k in 0..4 {
                dst[((y * dw + x) * 4) as usize + k] = ((acc[k] + 2) / 4) as u8;
            }
        }
    }
    Level { rgba: dst, w: dw, h: dh }
}

/// Plain 2x2 box-filtered chain down to 1x1.
pub fn box_chain(rgba: &[u8], w: u32, h: u32) -> Vec<Level> {
    let mut out = vec![Level { rgba: rgba[..(w * h * 4) as usize].to_vec(), w, h }];
    while out.last().is_some_and(|l| l.w > 1 || l.h > 1) {
        let l = out.last().unwrap();
        let next = halve(&l.rgba, l.w, l.h);
        out.push(next);
    }
    out
}

/// Copies RGB into fully transparent texels from the nearest texel with alpha > 0
/// (4-connected breadth-first search, no wrap). Alpha is never modified.
pub fn fill_transparent(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut out = rgba[..w * h * 4].to_vec();
    let mut done: Vec<bool> = (0..w * h).map(|i| out[i * 4 + 3] > 0).collect();
    let mut queue: std::collections::VecDeque<usize> = (0..w * h).filter(|&i| done[i]).collect();
    if queue.is_empty() {
        return out;
    }
    while let Some(i) = queue.pop_front() {
        let (x, y) = (i % w, i / w);
        let rgb = [out[i * 4], out[i * 4 + 1], out[i * 4 + 2]];
        let mut visit = |nx: usize, ny: usize| {
            let j = ny * w + nx;
            if !done[j] {
                done[j] = true;
                out[j * 4..j * 4 + 3].copy_from_slice(&rgb);
                queue.push_back(j);
            }
        };
        if x > 0 {
            visit(x - 1, y);
        }
        if x + 1 < w {
            visit(x + 1, y);
        }
        if y > 0 {
            visit(x, y - 1);
        }
        if y + 1 < h {
            visit(x, y + 1);
        }
    }
    out
}

/// Fraction of texels whose alpha reaches `CUTOUT_REF`.
pub fn coverage(rgba: &[u8]) -> f64 {
    let n = rgba.len() / 4;
    if n == 0 {
        return 0.0;
    }
    rgba.chunks_exact(4).filter(|p| p[3] >= CUTOUT_REF).count() as f64 / n as f64
}

fn scaled(a: u8, s: f32) -> u8 {
    (a as f32 * s).round().clamp(0.0, 255.0) as u8
}

/// Scale factor for `alpha` so that about `target` of the texels reach the reference,
/// chosen as close to 1 as the ties allow.
fn coverage_scale(alpha: &[u8], target: f64) -> f32 {
    let n = alpha.len();
    let k = (target * n as f64).round() as usize;
    if k == 0 || k > n {
        return 1.0;
    }
    let mut sorted = alpha.to_vec();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    let need = CUTOUT_REF as f32 - 0.5; // round(a * s) >= REF  <=>  a * s >= REF - 0.5
    let (ak, next) = (sorted[k - 1], sorted.get(k).copied());
    let count = |s: f32| alpha.iter().filter(|&&a| scaled(a, s) >= CUTOUT_REF).count();
    let mut candidates = vec![1.0f32];
    if ak > 0 {
        candidates.push(need / ak as f32 * 1.0001);
    } else {
        candidates.push(8.0);
    }
    if let Some(nx) = next.filter(|&v| v > 0) {
        candidates.push(need / nx as f32 * 0.9999);
    }
    let mut best = (usize::MAX, f32::MAX, 1.0f32);
    for s in candidates {
        let s = s.clamp(0.05, 8.0);
        let err = count(s).abs_diff(k);
        let dist = s.ln().abs();
        if err < best.0 || (err == best.0 && dist < best.1) {
            best = (err, dist, s);
        }
    }
    best.2
}

/// Coverage-preserving chain for alpha-tested textures.
pub fn cutout_chain(rgba: &[u8], w: u32, h: u32) -> Vec<Level> {
    let target = coverage(&rgba[..(w * h * 4) as usize]);
    // Filter from the edge-filled copy; level 0 alpha stays exactly the input alpha. Each level
    // is filtered from the previous unscaled level, then its alpha is scaled for output, so
    // scaling errors do not compound.
    let filled = fill_transparent(rgba, w, h);
    let mut raw = Level { rgba: filled.clone(), w, h };
    let mut out = vec![Level { rgba: filled, w, h }];
    while raw.w > 1 || raw.h > 1 {
        raw = halve(&raw.rgba, raw.w, raw.h);
        let mut level = Level { rgba: raw.rgba.clone(), w: raw.w, h: raw.h };
        let alpha: Vec<u8> = level.rgba.chunks_exact(4).map(|p| p[3]).collect();
        let s = coverage_scale(&alpha, target);
        if s != 1.0 {
            for p in level.rgba.chunks_exact_mut(4) {
                p[3] = scaled(p[3], s);
            }
        }
        out.push(level);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lcg(seed: &mut u32) -> u32 {
        *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        *seed >> 8
    }

    /// Thin "leaf" texture: sparse blobs and 1-texel stripes, black RGB where clear.
    fn leaves(w: u32, h: u32) -> Vec<u8> {
        let mut seed = 7u32;
        let mut px = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let on = (x % 5 == 0) || (y % 7 == 0 && x % 2 == 0) || lcg(&mut seed) % 10 == 0;
                let p = &mut px[((y * w + x) * 4) as usize..][..4];
                if on {
                    p.copy_from_slice(&[30 + (x % 50) as u8, 160, 40, if x % 3 == 0 { 200 } else { 255 }]);
                } else {
                    p.copy_from_slice(&[0, 0, 0, if x % 11 == 0 { 20 } else { 0 }]);
                }
            }
        }
        px
    }

    #[test]
    fn level0_alpha_is_untouched() {
        let src = leaves(64, 64);
        let c = cutout_chain(&src, 64, 64);
        let a0: Vec<u8> = c[0].rgba.chunks_exact(4).map(|p| p[3]).collect();
        let a_in: Vec<u8> = src.chunks_exact(4).map(|p| p[3]).collect();
        assert_eq!(a0, a_in);
        assert_eq!((c[0].w, c[0].h), (64, 64));
        assert_eq!(c.len(), 7);
    }

    #[test]
    fn coverage_is_preserved_per_level() {
        let src = leaves(64, 64);
        let target = coverage(&src);
        assert!(target > 0.1 && target < 0.9, "test texture should be partly covered, got {target}");
        let c = cutout_chain(&src, 64, 64);
        for l in &c[1..] {
            let n = (l.w * l.h) as f64;
            if n < 16.0 {
                continue;
            }
            let got = coverage(&l.rgba);
            assert!((got - target).abs() <= (0.02f64).max(1.5 / n), "level {}x{}: coverage {got} vs {target}", l.w, l.h);
        }
    }

    #[test]
    fn plain_box_filter_loses_coverage() {
        // Documents why the scaling exists: sparse single-texel dots (thin leaves, grass blades)
        // vanish in a box chain once their averaged alpha falls under the test reference.
        let (w, h) = (128u32, 128u32);
        let mut seed = 5u32;
        let mut src = vec![0u8; (w * h * 4) as usize];
        for p in src.chunks_exact_mut(4) {
            if lcg(&mut seed) % 100 < 12 {
                p[3] = 255;
            }
        }
        let target = coverage(&src);
        let boxed = box_chain(&src, w, h);
        let kept = cutout_chain(&src, w, h);
        for lvl in 2..5 {
            assert!(coverage(&boxed[lvl].rgba) < target * 0.5, "box level {lvl}");
            let n = (kept[lvl].w * kept[lvl].h) as f64;
            assert!((coverage(&kept[lvl].rgba) - target).abs() < 0.02f64.max(3.0 / n), "kept level {lvl}: {} vs {target}", coverage(&kept[lvl].rgba));
        }
    }

    #[test]
    fn opaque_texture_chain_equals_box_filter() {
        for (w, h) in [(32u32, 32u32), (12, 6), (1, 8)] {
            let mut seed = 99u32;
            let src: Vec<u8> = (0..w * h * 4).map(|i| if i % 4 == 3 { 255 } else { lcg(&mut seed) as u8 }).collect();
            let a = cutout_chain(&src, w, h);
            let b = box_chain(&src, w, h);
            assert_eq!(a.len(), b.len());
            for (x, y) in a.iter().zip(&b) {
                assert_eq!((x.w, x.h, &x.rgba), (y.w, y.h, &y.rgba), "{w}x{h}");
            }
            assert_eq!(classify(&src), TexClass::Opaque);
        }
    }

    #[test]
    fn transparent_texels_take_neighbor_color() {
        // 4x1: green opaque, clear black, clear black, red opaque.
        let src = [0u8, 255, 0, 255, 0, 0, 0, 0, 0, 0, 0, 0, 255, 0, 0, 255];
        let f = fill_transparent(&src, 4, 1);
        assert_eq!(&f[4..8], &[0, 255, 0, 0]);
        assert_eq!(&f[8..12], &[255, 0, 0, 0]);
        assert_eq!(f.iter().skip(3).step_by(4).copied().collect::<Vec<_>>(), vec![255, 0, 0, 255]);
        // Level 1 color of the green/clear pair is pure green, not darkened by black.
        let c = cutout_chain(&src, 4, 1);
        assert_eq!(&c[1].rgba[0..3], &[0, 255, 0]);
    }

    #[test]
    fn classify_examples() {
        let opaque = [10u8, 20, 30, 255].repeat(16);
        assert_eq!(classify(&opaque), TexClass::Opaque);
        let mut cut = opaque.clone();
        cut[3] = 0;
        cut[7] = 0;
        assert_eq!(classify(&cut), TexClass::Cutout);
        let glass = [10u8, 20, 30, 100].repeat(16);
        assert_eq!(classify(&glass), TexClass::Translucent);
    }
}
