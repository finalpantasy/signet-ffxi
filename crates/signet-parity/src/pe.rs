//! Minimal PE32 header reader (enough to find the image base, size and executable sections).

#[derive(Clone, Debug)]
pub struct Section {
    pub name: String,
    pub va: u32,
    pub vsize: u32,
    pub chars: u32,
}

#[derive(Clone, Debug)]
pub struct PeInfo {
    pub image_base: u32,
    pub size_of_image: u32,
    pub sections: Vec<Section>,
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

/// Parses the headers at the start of a memory-mapped image (or a file's first page).
pub fn parse(b: &[u8]) -> Result<PeInfo, String> {
    if b.get(0..2) != Some(b"MZ") {
        return Err("not a PE image (no MZ)".into());
    }
    let pe = u32_at(b, 0x3c).ok_or("short header")? as usize;
    if b.get(pe..pe + 4) != Some(b"PE\0\0") {
        return Err("not a PE image (no PE signature)".into());
    }
    let machine = u16_at(b, pe + 4).ok_or("short header")?;
    if machine != 0x14c {
        return Err(format!("not a 32-bit x86 image (machine {machine:#x})"));
    }
    let nsec = u16_at(b, pe + 6).ok_or("short header")? as usize;
    let opt_size = u16_at(b, pe + 20).ok_or("short header")? as usize;
    let opt = pe + 24;
    if u16_at(b, opt) != Some(0x10b) {
        return Err("not a PE32 optional header".into());
    }
    let image_base = u32_at(b, opt + 28).ok_or("short header")?;
    let size_of_image = u32_at(b, opt + 56).ok_or("short header")?;
    let mut sections = Vec::new();
    for i in 0..nsec {
        let o = opt + opt_size + i * 40;
        let name = b.get(o..o + 8).ok_or("short section table")?;
        let name = String::from_utf8_lossy(name).trim_end_matches('\0').to_string();
        sections.push(Section {
            name,
            va: u32_at(b, o + 12).ok_or("short section table")?,
            vsize: u32_at(b, o + 8).ok_or("short section table")?,
            chars: u32_at(b, o + 36).ok_or("short section table")?,
        });
    }
    Ok(PeInfo { image_base, size_of_image, sections })
}

impl PeInfo {
    /// RVA ranges of sections marked executable (IMAGE_SCN_MEM_EXECUTE or CNT_CODE). The packer's
    /// own `POL1` section holds compressed data, not code, and is skipped.
    pub fn code_ranges(&self) -> Vec<(u32, u32)> {
        self.sections.iter().filter(|s| s.chars & 0x2000_0020 != 0 && s.name != "POL1").map(|s| (s.va, s.va + s.vsize)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_garbage() {
        assert!(parse(b"hello").is_err());
        assert!(parse(&[0u8; 512]).is_err());
    }
}
