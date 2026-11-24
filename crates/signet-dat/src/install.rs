//! Locating DAT files in a local FFXI install.
//!
//! Each ROM folder n (1 = `ROM`, 2..=9 = `ROM2`..`ROM9`) has a VTABLE and an
//! FTABLE indexed by file id. A non-zero VTABLE byte means ROM n holds that id;
//! the FTABLE u16 then gives the path: directory = value >> 7, file = value & 0x7F,
//! i.e. `ROMn/<dir>/<file>.DAT`. ROM 1's tables sit in the install root.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const MAX_ROM: u8 = 9;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DatLocation {
    pub rom: u8,
    pub dir: u16,
    pub file: u8,
}

impl DatLocation {
    pub fn rom_folder(&self) -> String {
        if self.rom == 1 { "ROM".into() } else { format!("ROM{}", self.rom) }
    }

    pub fn relative_path(&self) -> PathBuf {
        Path::new(&self.rom_folder())
            .join(self.dir.to_string())
            .join(format!("{}.DAT", self.file))
    }
}

struct RomTable {
    rom: u8,
    vtable: Vec<u8>,
    ftable: Vec<u8>,
}

pub struct Install {
    root: PathBuf,
    tables: Vec<RomTable>,
}

impl Install {
    /// Opens the `FINAL FANTASY XI` folder. ROM folders without tables are skipped.
    pub fn open(root: impl Into<PathBuf>) -> io::Result<Self> {
        let root = root.into();
        let mut tables = Vec::new();
        for rom in 1..=MAX_ROM {
            let (v, f) = if rom == 1 {
                (root.join("VTABLE.DAT"), root.join("FTABLE.DAT"))
            } else {
                let dir = root.join(format!("ROM{rom}"));
                (dir.join(format!("VTABLE{rom}.DAT")), dir.join(format!("FTABLE{rom}.DAT")))
            };
            let vtable = match fs::read(&v) {
                Ok(b) => b,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            let ftable = fs::read(&f)?;
            if ftable.len() != vtable.len() * 2 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("ROM{rom}: FTABLE is {} bytes, expected {}", ftable.len(), vtable.len() * 2),
                ));
            }
            tables.push(RomTable { rom, vtable, ftable });
        }
        if tables.is_empty() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no VTABLE.DAT found"));
        }
        Ok(Self { root, tables })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// ROM numbers whose tables were found.
    pub fn roms(&self) -> impl Iterator<Item = u8> + '_ {
        self.tables.iter().map(|t| t.rom)
    }

    /// Largest file id any table covers, plus one.
    pub fn id_count(&self) -> u32 {
        self.tables.iter().map(|t| t.vtable.len() as u32).max().unwrap_or(0)
    }

    /// Every ROM that claims this id, lowest ROM first.
    pub fn claims(&self, id: u32) -> impl Iterator<Item = DatLocation> + '_ {
        self.tables.iter().filter_map(move |t| {
            let i = id as usize;
            if *t.vtable.get(i)? == 0 {
                return None;
            }
            let v = u16::from_le_bytes([t.ftable[2 * i], t.ftable[2 * i + 1]]);
            Some(DatLocation { rom: t.rom, dir: v >> 7, file: (v & 0x7f) as u8 })
        })
    }

    /// The location used for an id: the highest ROM that claims it.
    pub fn locate(&self, id: u32) -> Option<DatLocation> {
        self.claims(id).last()
    }

    pub fn path(&self, loc: DatLocation) -> PathBuf {
        self.root.join(loc.relative_path())
    }

    pub fn read(&self, id: u32) -> io::Result<Vec<u8>> {
        let loc = self
            .locate(id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("file id {id} not in any table")))?;
        fs::read(self.path(loc))
    }
}
