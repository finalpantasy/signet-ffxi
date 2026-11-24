//! Per-pixel diff of two RGBA PNGs written by signet-render (stored-deflate, filter 0).
//! Usage: signet-render-pngdiff a.png b.png [--out diff.png] [--gain N]
//! Prints differing pixel count, max channel delta, mean delta and a delta histogram. The diff
//! image is black where equal and a red-to-yellow ramp (delta times gain, default 16) where not.
use std::process::ExitCode;

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// Decodes the PNGs this project writes: 8-bit RGBA, no interlace, filter type 0, stored deflate.
fn decode(png: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    if png.len() < 8 || &png[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err("not a PNG".into());
    }
    let (mut p, mut w, mut h, mut idat) = (8usize, 0u32, 0u32, Vec::new());
    while p + 12 <= png.len() {
        let n = be32(&png[p..]) as usize;
        let kind = &png[p + 4..p + 8];
        let body = png.get(p + 8..p + 8 + n).ok_or("truncated chunk")?;
        match kind {
            b"IHDR" => {
                w = be32(body);
                h = be32(&body[4..]);
                if body[8..13] != [8, 6, 0, 0, 0] {
                    return Err("only 8-bit RGBA non-interlaced PNGs are supported".into());
                }
            }
            b"IDAT" => idat.extend_from_slice(body),
            _ => {}
        }
        p += 12 + n;
    }
    if idat.len() < 6 {
        return Err("no image data".into());
    }
    let (mut raw, mut q) = (Vec::new(), 2usize);
    loop {
        let hdr = *idat.get(q).ok_or("truncated deflate")?;
        if hdr & 6 != 0 {
            return Err("compressed deflate blocks are not supported (use PNGs from signet-render)".into());
        }
        let n = u16::from_le_bytes([idat[q + 1], idat[q + 2]]) as usize;
        raw.extend_from_slice(idat.get(q + 5..q + 5 + n).ok_or("truncated stored block")?);
        q += 5 + n;
        if hdr & 1 != 0 {
            break;
        }
    }
    let row = w as usize * 4;
    if raw.len() != (row + 1) * h as usize {
        return Err("size mismatch".into());
    }
    let mut rgba = Vec::with_capacity(row * h as usize);
    for y in 0..h as usize {
        if raw[y * (row + 1)] != 0 {
            return Err("filtered rows are not supported".into());
        }
        rgba.extend_from_slice(&raw[y * (row + 1) + 1..(y + 1) * (row + 1)]);
    }
    Ok((w, h, rgba))
}

struct Stats {
    differing: u64,
    total: u64,
    max_delta: u8,
    sum_delta: u64,
    /// Pixels whose max channel delta exceeds 0, 2, 8, 32.
    over: [u64; 4],
    first: Option<(u32, u32)>,
    worst: (u32, u32),
}

/// Compares RGB (alpha is forced to 255 by the renderer) and builds the diff image.
fn diff(w: u32, a: &[u8], b: &[u8], gain: u32) -> (Stats, Vec<u8>) {
    let mut s = Stats { differing: 0, total: (a.len() / 4) as u64, max_delta: 0, sum_delta: 0, over: [0; 4], first: None, worst: (0, 0) };
    let mut img = vec![255u8; a.len()];
    for (i, (pa, pb)) in a.chunks_exact(4).zip(b.chunks_exact(4)).enumerate() {
        let d = (0..3).map(|c| pa[c].abs_diff(pb[c])).max().unwrap();
        let o = &mut img[i * 4..i * 4 + 4];
        let (x, y) = ((i as u32) % w, (i as u32) / w);
        if d == 0 {
            o[..3].copy_from_slice(&[0, 0, 0]);
            continue;
        }
        s.differing += 1;
        s.sum_delta += d as u64;
        for (k, t) in [0u8, 2, 8, 32].iter().enumerate() {
            if d > *t {
                s.over[k] += 1;
            }
        }
        s.first.get_or_insert((x, y));
        if d > s.max_delta {
            s.max_delta = d;
            s.worst = (x, y);
        }
        let v = (d as u32 * gain).min(510);
        o[0] = 255.min(v) as u8;
        o[1] = v.saturating_sub(255) as u8;
        o[2] = 0;
    }
    (s, img)
}

fn main() -> ExitCode {
    match run() {
        Ok(same) => if same { ExitCode::SUCCESS } else { ExitCode::from(2) },
        Err(e) => {
            eprintln!("signet-render-pngdiff: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<bool, String> {
    let (mut files, mut out, mut gain) = (Vec::new(), None, 16u32);
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => out = Some(it.next().ok_or("--out needs a path")?),
            "--gain" => gain = it.next().and_then(|v| v.parse().ok()).ok_or("--gain needs a number")?,
            _ => files.push(a),
        }
    }
    if files.len() != 2 {
        return Err("usage: signet-render-pngdiff a.png b.png [--out diff.png] [--gain N]".into());
    }
    let load = |p: &str| std::fs::read(p).map_err(|e| format!("read {p}: {e}")).and_then(|b| decode(&b).map_err(|e| format!("{p}: {e}")));
    let (wa, ha, a) = load(&files[0])?;
    let (wb, hb, b) = load(&files[1])?;
    if (wa, ha) != (wb, hb) {
        return Err(format!("size differs: {wa}x{ha} vs {wb}x{hb}"));
    }
    let (s, img) = diff(wa, &a, &b, gain);
    println!(
        "{}x{}  differing pixels {} of {} ({:.4}%)  max channel delta {}  mean delta (differing) {:.2}",
        wa,
        ha,
        s.differing,
        s.total,
        s.differing as f64 * 100.0 / s.total as f64,
        s.max_delta,
        if s.differing > 0 { s.sum_delta as f64 / s.differing as f64 } else { 0.0 }
    );
    println!("pixels with delta >0: {}  >2: {}  >8: {}  >32: {}", s.over[0], s.over[1], s.over[2], s.over[3]);
    if let Some((x, y)) = s.first {
        println!("first differing pixel ({x},{y}); worst pixel ({},{})", s.worst.0, s.worst.1);
    }
    if let Some(p) = out {
        std::fs::write(&p, signet_dat::png::encode_rgba(wa, ha, &img)).map_err(|e| format!("write {p}: {e}"))?;
        println!("wrote {p}");
    }
    Ok(s.differing == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_diff() {
        let (w, h) = (3u32, 2u32);
        let a: Vec<u8> = (0..w * h * 4).map(|i| (i * 7) as u8).collect();
        let mut b = a.clone();
        b[4] = b[4].wrapping_add(5); // pixel 1, red
        let (dw, dh, da) = decode(&signet_dat::png::encode_rgba(w, h, &a)).unwrap();
        assert_eq!((dw, dh, &da), (w, h, &a));
        let (s, img) = diff(w, &a, &b, 16);
        assert_eq!((s.differing, s.max_delta, s.first), (1, 5, Some((1, 0))));
        assert_eq!(&img[4..8], &[80, 0, 0, 255]);
        assert_eq!(diff(w, &a, &a, 16).0.differing, 0);
    }
}
