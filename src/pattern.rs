//! Bounded i686 code-signature matching. No process access or dereferencing of
//! game data is performed here. Only pointers encoded inside the supplied
//! executable-code slice can be resolved.
#[derive(Clone, Debug)]
pub struct Pattern {
    bytes: Vec<Option<u8>>,
    offset: usize,
    pointer: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub offset: usize,
    pub address: u32,
    pub resolved: u32,
}
impl Pattern {
    /// Compact hex/?? signatures. & selects an address; * selects a u32
    /// operand. With no marker the u32 immediately after the pattern is read.
    pub fn parse(source: &str) -> Result<Self, String> {
        let compact = source
            .bytes()
            .filter(|byte| !byte.is_ascii_whitespace())
            .collect::<Vec<_>>();
        let mut bytes = Vec::new();
        let mut marker = None;
        let mut index = 0;
        while index < compact.len() {
            let first = compact[index];
            if first == b'&' || first == b'*' {
                if marker.is_some() {
                    return Err("A signature may have only one result marker".into());
                }
                marker = Some((bytes.len(), first == b'*'));
                index += 1;
                continue;
            }
            let second = *compact
                .get(index + 1)
                .ok_or("Signature has an incomplete byte")?;
            if first == b'?' && second == b'?' {
                bytes.push(None);
            } else {
                let high = (first as char)
                    .to_digit(16)
                    .ok_or("Invalid signature byte")?;
                let low = (second as char)
                    .to_digit(16)
                    .ok_or("Invalid signature byte")?;
                bytes.push(Some((high * 16 + low) as u8));
            }
            if bytes.len() > 256 {
                return Err("Signature exceeds 256 bytes".into());
            }
            index += 2;
        }
        if bytes.is_empty() || bytes.iter().all(Option::is_none) {
            return Err("Signature must contain a fixed byte".into());
        }
        let (offset, pointer) = marker.unwrap_or((bytes.len(), true));
        Ok(Self {
            bytes,
            offset,
            pointer,
        })
    }
    pub fn matches(&self, data: &[u8], base: u32, limit: usize) -> Result<Vec<Match>, String> {
        if limit == 0 {
            return Err("A positive match limit is required".into());
        }
        let mut matches = Vec::new();
        if self.bytes.len() > data.len() {
            return Ok(matches);
        }
        let (anchor, byte) = self
            .bytes
            .iter()
            .enumerate()
            .find_map(|(index, byte)| byte.map(|value| (index, value)))
            .ok_or("Missing fixed signature byte")?;
        for start in 0..=data.len() - self.bytes.len() {
            if data[start + anchor] != byte {
                continue;
            }
            if !self
                .bytes
                .iter()
                .zip(&data[start..])
                .all(|(expected, observed)| expected.is_none_or(|value| value == *observed))
            {
                continue;
            }
            let offset = start
                .checked_add(self.offset)
                .ok_or("Signature result overflow")?;
            let address = base
                .checked_add(u32::try_from(start).map_err(|_| "Code offset exceeds i686")?)
                .ok_or("Code address exceeds i686")?;
            let resolved = if self.pointer {
                let operand = data
                    .get(offset..offset.checked_add(4).ok_or("Operand overflow")?)
                    .ok_or("Signature operand is outside the captured code")?;
                u32::from_le_bytes(operand.try_into().map_err(|_| "Invalid i686 operand")?)
            } else {
                base.checked_add(u32::try_from(offset).map_err(|_| "Code offset exceeds i686")?)
                    .ok_or("Code address exceeds i686")?
            };
            matches.push(Match {
                offset: start,
                address,
                resolved,
            });
            if matches.len() == limit {
                break;
            }
        }
        Ok(matches)
    }
    pub fn unique(&self, data: &[u8], base: u32) -> Result<Match, String> {
        let matches = self.matches(data, base, 2)?;
        match matches.len() {
            0 => Err("Signature was not found".into()),
            1 => Ok(matches[0].clone()),
            _ => Err("Signature is ambiguous".into()),
        }
    }
}

// ABI location signatures from Windower/Fenestra, commit
// e5bfb6442f49bdb4d31859bff6218bbeb1bea620, hooks/ffximain.cpp.
// See THIRD_PARTY_NOTICES.md for the upstream copyright/permission notice.
pub const FFXI_SIGNATURES: &[(&str, &str)] = &[
    ("chat_log_ptr", "884C2409884C240A518D4C240C518B0D"),
    (
        "chat_mode_ptr",
        "8D4C24086A006A016A0050518B0D????????E8????????8B0D",
    ),
    (
        "autotranslate_ptr",
        "8BFE83C9FF33C06A00F2AEF7D1496A01518B0D&",
    ),
    ("menu_ptr", "5355568B30A0"),
    ("add_to_chat", "&81EC44080000535556578BE96A086A00"),
    ("input_command", "&81EC040400008B8C2408040000555657"),
    (
        "decode_packet",
        "&8B44241881ECB805000053558BAC24C8050000563BE857",
    ),
    (
        "encode_packet",
        "&518B4C24145333C0558B6C24144956578D14CD0000000033FF85ED89542410",
    ),
    (
        "autotranslate_lookup",
        "&5153558B6C24145685ED578BF1C644241000C644241100C64424",
    ),
    ("draw_scene", "&568BF18B86500D0000"),
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn address_marker_and_last_possible_position() {
        assert_eq!(
            Pattern::parse("&AA ?? CC")
                .unwrap()
                .unique(&[0, 0xaa, 0x55, 0xcc], 0x1000)
                .unwrap(),
            Match {
                offset: 1,
                address: 0x1001,
                resolved: 0x1001
            }
        );
        assert_eq!(
            Pattern::parse("AA&BB")
                .unwrap()
                .unique(&[0xaa, 0xbb], 0x1000)
                .unwrap()
                .resolved,
            0x1001
        );
    }
    #[test]
    fn resolves_only_code_embedded_operands() {
        assert_eq!(
            Pattern::parse("8B0D")
                .unwrap()
                .unique(&[0x8b, 0x0d, 0x78, 0x56, 0x34, 0x12], 0x1000)
                .unwrap()
                .resolved,
            0x12345678
        );
        assert!(
            Pattern::parse("8B0D")
                .unwrap()
                .unique(&[0x8b, 0x0d], 0x1000)
                .is_err()
        );
    }
    #[test]
    fn rejects_ambiguous_missing_and_malformed_signatures() {
        assert!(
            Pattern::parse("&AA")
                .unwrap()
                .unique(&[0xaa, 0xaa], 0)
                .is_err()
        );
        assert!(Pattern::parse("&AA").unwrap().unique(&[0], 0).is_err());
        for bad in ["", "??", "A?", "GG", "AA&*BB", "A"] {
            assert!(Pattern::parse(bad).is_err(), "{bad}");
        }
        assert!(
            Pattern::parse("&AA")
                .unwrap()
                .matches(&[0xaa], 0, 0)
                .is_err()
        );
    }
    #[test]
    fn parses_all_frozen_game_signatures() {
        for (_, source) in FFXI_SIGNATURES {
            Pattern::parse(source).unwrap();
        }
    }
}
