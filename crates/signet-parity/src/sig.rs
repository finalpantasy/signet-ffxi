//! Byte-pattern signatures: parse, scan, capture operands, and demand a unique (or consistent)
//! match.

#[derive(Clone, Debug)]
pub struct Pattern {
    bytes: Vec<u8>,
    mask: Vec<bool>,
}

impl Pattern {
    /// `"a1 ?? ?? ?? ?? 8b 40 50 c3"`: hex bytes, `??` is any byte.
    pub fn parse(s: &str) -> Result<Pattern, String> {
        let mut bytes = Vec::new();
        let mut mask = Vec::new();
        for t in s.split_whitespace() {
            if t == "??" {
                bytes.push(0);
                mask.push(false);
            } else {
                bytes.push(u8::from_str_radix(t, 16).map_err(|_| format!("bad pattern byte `{t}`"))?);
                mask.push(true);
            }
        }
        if bytes.is_empty() || !mask[0] {
            return Err("pattern must start with a fixed byte".into());
        }
        Ok(Pattern { bytes, mask })
    }
    pub fn len(&self) -> usize {
        self.bytes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
    fn matches_at(&self, h: &[u8], i: usize) -> bool {
        for (k, (&b, &m)) in self.bytes.iter().zip(&self.mask).enumerate() {
            if m && h[i + k] != b {
                return false;
            }
        }
        true
    }
    /// Offsets of every match whose first byte lies in `[lo, hi)`.
    pub fn scan(&self, h: &[u8], lo: usize, hi: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let n = self.bytes.len();
        if h.len() < n {
            return out;
        }
        let end = hi.min(h.len() - n + 1);
        let first = self.bytes[0];
        let mut i = lo;
        while i < end {
            match h[i..end].iter().position(|&b| b == first) {
                None => break,
                Some(p) => {
                    i += p;
                    if self.matches_at(h, i) {
                        out.push(i);
                    }
                    i += 1;
                }
            }
        }
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    U8,
    U32,
}

/// A value read from the matched bytes: `at` is the byte offset inside the pattern.
#[derive(Clone, Copy, Debug)]
pub struct Cap {
    pub name: &'static str,
    pub at: usize,
    pub width: Width,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Exactly one match.
    Unique,
    /// One or more matches that all capture the same values (a helper copied to many call sites).
    Consensus,
}

#[derive(Clone, Copy, Debug)]
pub struct Sig {
    pub name: &'static str,
    pub pattern: &'static str,
    pub caps: &'static [Cap],
    pub mode: Mode,
}

#[derive(Clone, Debug)]
pub struct Hit {
    pub sig: &'static str,
    /// Image offsets (RVAs) of every match.
    pub at: Vec<usize>,
    pub values: Vec<(&'static str, u32)>,
}

impl Hit {
    pub fn get(&self, name: &str) -> Option<u32> {
        self.values.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
    }
}

fn read_cap(h: &[u8], at: usize, c: &Cap) -> u32 {
    match c.width {
        Width::U8 => h[at + c.at] as u32,
        Width::U32 => u32::from_le_bytes(h[at + c.at..at + c.at + 4].try_into().unwrap()),
    }
}

/// Scans the code ranges (RVA pairs) of `image` for `sig`.
pub fn resolve(sig: &Sig, image: &[u8], ranges: &[(u32, u32)]) -> Result<Hit, String> {
    let p = Pattern::parse(sig.pattern)?;
    let mut at = Vec::new();
    for &(lo, hi) in ranges {
        at.extend(p.scan(image, lo as usize, hi as usize));
    }
    if at.is_empty() {
        return Err(format!("{}: no match", sig.name));
    }
    if sig.mode == Mode::Unique && at.len() != 1 {
        return Err(format!("{}: {} matches (need exactly 1) at {}", sig.name, at.len(), fmt_at(&at)));
    }
    let first: Vec<(&'static str, u32)> = sig.caps.iter().map(|c| (c.name, read_cap(image, at[0], c))).collect();
    for &a in &at[1..] {
        for (c, (_, v0)) in sig.caps.iter().zip(&first) {
            let v = read_cap(image, a, c);
            if v != *v0 {
                return Err(format!("{}: sites disagree on `{}` ({:#x} at {:#x} vs {:#x} at {:#x})", sig.name, c.name, v0, at[0], v, a));
            }
        }
    }
    Ok(Hit { sig: sig.name, at, values: first })
}

fn fmt_at(at: &[usize]) -> String {
    at.iter().take(6).map(|a| format!("{a:#x}")).collect::<Vec<_>>().join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_scan_and_captures() {
        let img = [0x00, 0xa1, 0x78, 0x56, 0x34, 0x12, 0x8b, 0x40, 0x50, 0xc3, 0xa1, 0x78, 0x56, 0x34, 0x12, 0x8b, 0x40, 0x50, 0xc3];
        let p = Pattern::parse("a1 ?? ?? ?? ?? 8b 40 50 c3").unwrap();
        assert_eq!(p.scan(&img, 0, img.len()), vec![1, 10]);
        static CAPS: [Cap; 2] = [Cap { name: "g", at: 1, width: Width::U32 }, Cap { name: "o", at: 7, width: Width::U8 }];
        let s = Sig { name: "t", pattern: "a1 ?? ?? ?? ?? 8b 40 50 c3", caps: &CAPS, mode: Mode::Consensus };
        let h = resolve(&s, &img, &[(0, img.len() as u32)]).unwrap();
        assert_eq!(h.get("g"), Some(0x12345678));
        assert_eq!(h.get("o"), Some(0x50));
        let u = Sig { mode: Mode::Unique, ..s };
        assert!(resolve(&u, &img, &[(0, img.len() as u32)]).unwrap_err().contains("2 matches"));
    }

    #[test]
    fn consensus_rejects_disagreement() {
        let img = [0xa1, 1, 0, 0, 0, 0xc3, 0xa1, 2, 0, 0, 0, 0xc3];
        static CAPS: [Cap; 1] = [Cap { name: "g", at: 1, width: Width::U32 }];
        let s = Sig { name: "t", pattern: "a1 ?? ?? ?? ?? c3", caps: &CAPS, mode: Mode::Consensus };
        assert!(resolve(&s, &img, &[(0, 12)]).unwrap_err().contains("disagree"));
    }

    #[test]
    fn bad_patterns() {
        assert!(Pattern::parse("?? 01").is_err());
        assert!(Pattern::parse("zz").is_err());
    }
}
