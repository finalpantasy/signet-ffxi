//! Zone id and name lookup, read from the user's own install at runtime.
//!
//! Findings (original work, checked against the install):
//! - Area names live in an `XISTRING` file (file id [`NAMES_FILE_ID`], `ROM\97\53.DAT` here).
//!   Layout: 8-byte magic `XISTRING`, then at `0x20` four u32s `[file size, entry count,
//!   entry table bytes, string data bytes]`; 8 more bytes follow (a stamp), so the entry
//!   table starts at `0x38` with 12-byte entries `[offset, length, flags]`, and the NUL-
//!   terminated strings start at `0x38 + table bytes`. There is no obfuscation. The table
//!   has 256 entries and the index equals the zone id (230 = Southern San d'Oria).
//!   The file ids just after it hold the short English names, the Japanese names and a
//!   second English variant; those tables stop at id 255.
//! - Names for ids 256 and up (Adoulin, Rala Waterways, Escha, Reisenjima, Mog Garden ...) are
//!   a static table inside `FFXiMain.dll` in the install root: an array indexed by zone id of
//!   12-byte entries `[long name ptr, 0, short name ptr]` (32-bit image, pointers are virtual
//!   addresses into `.rdata`). Index 100 is West Ronfaure and 230 Southern San d'Oria, matching
//!   the XISTRING table, so the index is the zone id; unused ids hold null pointers. It is found
//!   at runtime by looking for the entry that points at "Southern San d'Oria" / "SSandOria",
//!   which sits at index 230, and walking from there (see [`parse_dll_names`]). The array ends
//!   after id 298 here (Walk of Echoes [P]). Auto-translate "Place Names 2" (`ROM\168\25.DAT`) holds only a subset in its own order, so it is not used.
//! - Zone model: the zone's DAT (MZB layout, MMB meshes, textures, and the collision block
//!   inside the MZB) is file id `100 + id` for ids 0..=255 and `83635 + id` above (so 256 is file 83891; the
//!   task note's `83635 + (id - 256)` lands on 592-byte placeholder files here, ids 83635..=83890
//!   are an unrelated set; checked against the 7 to 13 MB zone-sized files starting at 83891).
//! - Dialog text: `6420 + id`, then `85335 + id` (256 is 85591, ~1.4 MB). NPC list: `6720 + id`,
//!   then `86490 + id` (unverified: nothing exists at 86746 here, so ids 256+ have no NPC list).

use std::path::PathBuf;

use crate::install::Install;

/// File id of the English area-name table.
pub const NAMES_FILE_ID: u32 = 65;
/// Highest zone id probed.
pub const MAX_ZONE_ID: u16 = 1023;

/// File id of a zone's model DAT.
pub fn model_file_id(zone: u16) -> u32 {
    if zone < 256 { 100 + zone as u32 } else { 83635 + zone as u32 }
}
/// File id of a zone's dialog string table.
pub fn dialog_file_id(zone: u16) -> u32 {
    if zone < 256 { 6420 + zone as u32 } else { 85335 + zone as u32 }
}
/// File id of a zone's NPC list.
pub fn npc_file_id(zone: u16) -> u32 {
    if zone < 256 { 6720 + zone as u32 } else { 86490 + zone as u32 }
}

/// Parses an `XISTRING` table into its strings (bytes as Latin-1; names are ASCII).
pub fn parse_string_table(b: &[u8]) -> Option<Vec<String>> {
    if b.get(..8)? != b"XISTRING" {
        return None;
    }
    let u32at = |o: usize| b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize);
    let count = u32at(0x24)?;
    let table = u32at(0x28)?;
    if table != count.checked_mul(12)? {
        return None;
    }
    let data = 0x38usize.checked_add(table)?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let e = 0x38 + 12 * i;
        let (off, len) = (u32at(e)?, u32at(e + 4)?);
        let start = data.checked_add(off)?;
        let s = b.get(start..start.checked_add(len)?)?;
        let s = s.split(|&c| c == 0).next().unwrap_or(&[]);
        out.push(s.iter().map(|&c| c as char).collect());
    }
    Some(out)
}

fn le32(b: &[u8], o: usize) -> Option<usize> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize)
}

/// Reads the zone-name array out of `FFXiMain.dll` bytes: `names[id]` is the long name (empty
/// for unused ids). Returns `None` when the PE or the array cannot be found and verified.
pub fn parse_dll_names(b: &[u8]) -> Option<Vec<String>> {
    if b.get(..2)? != b"MZ" {
        return None;
    }
    let pe = le32(b, 0x3c)?;
    if b.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let nsec = b.get(pe + 6..pe + 8).map(|s| u16::from_le_bytes([s[0], s[1]]) as usize)?;
    let opt = b.get(pe + 20..pe + 22).map(|s| u16::from_le_bytes([s[0], s[1]]) as usize)?;
    let image_base = le32(b, pe + 24 + 28)?;
    // (rva, virtual size, file offset, raw size)
    let mut secs = Vec::new();
    for i in 0..nsec {
        let o = pe + 24 + opt + 40 * i;
        secs.push((le32(b, o + 12)?, le32(b, o + 8)?, le32(b, o + 20)?, le32(b, o + 16)?));
    }
    let va_to_off = |va: usize| -> Option<usize> {
        let rva = va.checked_sub(image_base)?;
        secs.iter().find(|s| rva >= s.0 && rva < s.0 + s.1.min(s.3)).map(|s| s.2 + rva - s.0)
    };
    let off_to_va = |off: usize| -> Option<usize> {
        secs.iter().find(|s| off >= s.2 && off < s.2 + s.3).map(|s| image_base + s.0 + off - s.2)
    };
    let cstr = |va: usize| -> Option<String> {
        let o = va_to_off(va)?;
        let e = b.get(o..)?.iter().position(|&c| c == 0)?;
        let t = &b[o..o + e];
        (!t.is_empty() && t.len() < 64 && t.iter().all(|&c| (0x20..0x7f).contains(&c))).then(|| t.iter().map(|&c| c as char).collect())
    };
    let find = |needle: &[u8]| b.windows(needle.len()).position(|w| w == needle);
    // Anchor on id 230's entry.
    let long = off_to_va(find(b"Southern San d'Oria\0")?)?;
    let short = off_to_va(find(b"\0SSandOria\0")? + 1)?;
    let mut pat = Vec::new();
    for v in [long as u32, 0, short as u32] {
        pat.extend(v.to_le_bytes());
    }
    let hit = b.windows(12).position(|w| w == pat.as_slice())?;
    let base = hit.checked_sub(230 * 12)?;
    let mut out = Vec::new();
    for id in 0..=MAX_ZONE_ID as usize {
        let (Some(l), Some(z), Some(sh)) = (le32(b, base + id * 12), le32(b, base + id * 12 + 4), le32(b, base + id * 12 + 8)) else { break };
        if l == 0 && sh == 0 {
            out.push(String::new());
            continue;
        }
        match (cstr(l), z, cstr(sh)) {
            (Some(n), 0, Some(_)) => out.push(n),
            _ => break,
        }
    }
    (out.get(100).map(String::as_str) == Some("West Ronfaure") && out.get(230).is_some()).then_some(out)
}

#[derive(Debug, Clone)]
pub struct Zone {
    pub id: u16,
    /// Empty when the install has no name for this id.
    pub name: String,
    /// The zone's model DAT.
    pub model: PathBuf,
    pub dialog: Option<PathBuf>,
    pub npcs: Option<PathBuf>,
}

/// Result of [`Zones::resolve`].
#[derive(Debug)]
pub enum Resolved<'a> {
    Found(&'a Zone),
    Ambiguous(Vec<&'a Zone>),
    None,
}

pub struct Zones {
    zones: Vec<Zone>,
}

impl Zones {
    /// Reads names and file tables from the install at `root`.
    pub fn from_install(root: impl Into<PathBuf>) -> std::io::Result<Self> {
        let install = Install::open(root)?;
        let mut names = Self::read_names(&install);
        // The XISTRING table stops at 255; the client DLL's array covers the rest.
        if let Ok(dll) = std::fs::read(install.root().join("FFXiMain.dll"))
            && let Some(extra) = parse_dll_names(&dll)
        {
            if names.len() < extra.len() {
                names.resize(extra.len(), String::new());
            }
            for (i, n) in extra.into_iter().enumerate() {
                if names[i].is_empty() || names[i].eq_ignore_ascii_case("unknown") || names[i].eq_ignore_ascii_case("none") {
                    names[i] = n;
                }
            }
        }
        let path = |id: u32| install.locate(id).map(|l| install.path(l)).filter(|p| p.exists());
        let mut zones = Vec::new();
        for id in 0..=MAX_ZONE_ID {
            let Some(model) = path(model_file_id(id)) else { continue };
            let name = names.get(id as usize).cloned().unwrap_or_default();
            let name = if name.eq_ignore_ascii_case("none") || name.eq_ignore_ascii_case("unknown") { String::new() } else { name };
            zones.push(Zone { id, name, model, dialog: path(dialog_file_id(id)), npcs: path(npc_file_id(id)) });
        }
        Ok(Self { zones })
    }

    fn read_names(install: &Install) -> Vec<String> {
        // Preferred id first; the neighbors cover a shifted table. Accept the table whose
        // entry 230 mentions San d'Oria.
        for id in std::iter::once(NAMES_FILE_ID).chain(60..90) {
            if let Ok(b) = install.read(id)
                && let Some(t) = parse_string_table(&b)
                && t.get(230).is_some_and(|n| n.contains("San d'Oria") && n.len() > 10)
            {
                return t;
            }
        }
        Vec::new()
    }

    /// Resolves a user string: a zone id, an exact name (case-insensitive), a unique name prefix,
    /// or a unique substring, in that order.
    pub fn resolve(&self, q: &str) -> Resolved<'_> {
        let q = q.trim();
        if let Ok(id) = q.parse::<u16>() {
            return self.by_id(id).map_or(Resolved::None, Resolved::Found);
        }
        let q = q.to_ascii_lowercase();
        if q.is_empty() {
            return Resolved::None;
        }
        let named = || self.zones.iter().filter(|z| !z.name.is_empty());
        if let Some(z) = named().find(|z| z.name.to_ascii_lowercase() == q) {
            return Resolved::Found(z);
        }
        for hit in [|n: &str, q: &str| n.starts_with(q), |n: &str, q: &str| n.contains(q)] {
            let v: Vec<&Zone> = named().filter(|z| hit(&z.name.to_ascii_lowercase(), &q)).collect();
            match v.len() {
                0 => {}
                1 => return Resolved::Found(v[0]),
                _ => return Resolved::Ambiguous(v),
            }
        }
        Resolved::None
    }
    pub fn iter(&self) -> impl Iterator<Item = &Zone> {
        self.zones.iter()
    }
    pub fn len(&self) -> usize {
        self.zones.len()
    }
    pub fn is_empty(&self) -> bool {
        self.zones.is_empty()
    }
    pub fn by_id(&self, id: u16) -> Option<&Zone> {
        self.zones.binary_search_by_key(&id, |z| z.id).ok().map(|i| &self.zones[i])
    }
    /// Case-insensitive; an exact match wins, else the first zone whose name contains `name`.
    pub fn by_name(&self, name: &str) -> Option<&Zone> {
        let q = name.trim().to_ascii_lowercase();
        if q.is_empty() {
            return None;
        }
        self.zones
            .iter()
            .find(|z| z.name.to_ascii_lowercase() == q)
            .or_else(|| self.zones.iter().find(|z| z.name.to_ascii_lowercase().contains(&q)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(strings: &[&str]) -> Vec<u8> {
        let mut data = Vec::new();
        let mut ent = Vec::new();
        for s in strings {
            ent.extend((data.len() as u32).to_le_bytes());
            ent.extend((s.len() as u32 + 1).to_le_bytes());
            ent.extend(0u32.to_le_bytes());
            data.extend(s.as_bytes());
            data.push(0);
        }
        let mut b = b"XISTRING\0".to_vec();
        b.resize(0x20, 0);
        let total = 0x38 + ent.len() + data.len();
        for v in [total, strings.len(), ent.len(), data.len()] {
            b.extend((v as u32).to_le_bytes());
        }
        b.extend([0u8; 8]);
        b.extend(ent);
        b.extend(data);
        b
    }

    #[test]
    fn parses_synthetic_table() {
        let t = parse_string_table(&table(&["unknown", "Bastok Markets", "", "Norg"])).unwrap();
        assert_eq!(t, ["unknown", "Bastok Markets", "", "Norg"]);
    }

    #[test]
    fn rejects_bad_tables() {
        assert!(parse_string_table(b"nope").is_none());
        let mut t = table(&["a", "b"]);
        t.truncate(t.len() - 3);
        assert!(parse_string_table(&t).is_none());
        let mut t = table(&["a", "b"]);
        t[0x28] = 1; // table bytes no longer count * 12
        assert!(parse_string_table(&t).is_none());
    }

    #[test]
    fn file_id_rule() {
        assert_eq!(model_file_id(100), 200);
        assert_eq!(model_file_id(256), 83891);
        assert_eq!(dialog_file_id(257), 85592);
        assert_eq!(npc_file_id(0), 6720);
    }

    #[test]
    fn lookup_by_name() {
        let z = |id, name: &str| Zone { id, name: name.into(), model: PathBuf::new(), dialog: None, npcs: None };
        let zs = Zones { zones: vec![z(230, "Southern San d'Oria"), z(231, "Northern San d'Oria"), z(235, "Bastok Markets")] };
        assert_eq!(zs.by_name("bastok markets").unwrap().id, 235);
        assert_eq!(zs.by_name("northern").unwrap().id, 231);
        assert_eq!(zs.by_id(230).unwrap().name, "Southern San d'Oria");
        assert!(zs.by_name("").is_none() && zs.by_id(1).is_none());
    }

    #[test]
    fn resolve_rules() {
        let z = |id, name: &str| Zone { id, name: name.into(), model: PathBuf::new(), dialog: None, npcs: None };
        let zs = Zones { zones: vec![z(5, ""), z(100, "West Ronfaure"), z(101, "East Ronfaure"), z(258, "Rala Waterways"), z(259, "Rala Waterways [U]")] };
        let id = |r: Resolved| match r { Resolved::Found(z) => z.id as i32, Resolved::Ambiguous(v) => -(v.len() as i32), Resolved::None => 0 };
        assert_eq!(id(zs.resolve("WEST ronfaure")), 100);
        assert_eq!(id(zs.resolve("west")), 100);
        assert_eq!(id(zs.resolve("259")), 259);
        assert_eq!(id(zs.resolve("rala waterways")), 258);
        assert_eq!(id(zs.resolve("rala")), -2);
        assert_eq!(id(zs.resolve("ronfaure")), -2);
        assert_eq!(id(zs.resolve("nowhere")), 0);
        assert_eq!(id(zs.resolve("5")), 5); // unnamed zones resolve by id only
    }

    #[test]
    fn install_known_names() {
        let Some(root) = crate::find_install_root() else { return };
        let zs = Zones::from_install(root).unwrap();
        for (id, name) in [(256, "Western Adoulin"), (258, "Rala Waterways"), (280, "Mog Garden"), (100, "West Ronfaure"), (230, "Southern San d'Oria"), (234, "Bastok Mines"), (238, "Windurst Waters"), (245, "Lower Jeuno")] {
            assert_eq!(zs.by_id(id).map(|z| z.name.as_str()), Some(name), "zone {id}");
        }
        assert_eq!(zs.by_name("port windurst").unwrap().id, 240);
    }
}
