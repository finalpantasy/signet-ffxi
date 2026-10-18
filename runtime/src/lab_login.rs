//! Slot-zero navigation for the exact audited local client, on its draw thread.
//! Menu layout/signatures are documented by pinned Ashita autologin 1.2 and
//! config/ashita/ashita.pointers.ini. No original runtime is loaded.
use std::{
    ffi::c_void,
    sync::atomic::{AtomicU32, Ordering},
};
use windower_rs::{client_code::ScanReport, pattern::Pattern};
use windows_sys::Win32::System::{
    Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory},
    Threading::GetCurrentProcess,
};

fn read(address: usize, length: usize) -> Result<Vec<u8>, String> {
    if address < 0x10000 || address.checked_add(length).is_none() {
        return Err("Invalid local menu address".into());
    }
    let mut bytes = vec![0; length];
    let mut copied = 0;
    if unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as _,
            bytes.as_mut_ptr().cast(),
            length,
            &mut copied,
        )
    } == 0
        || copied != length
    {
        return Err("Local menu memory is not readable".into());
    }
    Ok(bytes)
}
fn number(address: usize) -> Result<usize, String> {
    Ok(u32::from_le_bytes(read(address, 4)?.try_into().unwrap()) as usize)
}
fn put(address: usize, value: u32) -> Result<(), String> {
    let mut written = 0;
    if unsafe {
        WriteProcessMemory(
            GetCurrentProcess(),
            address as *mut c_void,
            value.to_le_bytes().as_ptr().cast(),
            4,
            &mut written,
        )
    } == 0
        || written != 4
    {
        return Err("Local menu field is not writable".into());
    }
    Ok(())
}
pub struct Login {
    pointers: [usize; 6],
    code_ranges: Vec<(usize, usize)>,
    steps: AtomicU32,
}
impl Login {
    #[cfg(test)]
    pub(crate) fn fixture_done() -> Self {
        Self {
            pointers: [0; 6],
            code_ranges: Vec::new(),
            steps: AtomicU32::new(15),
        }
    }
    /// Caller has already checked the exact client hash, image path and absence
    /// of original cores. Only the private loopback xiloader trial may call this.
    pub fn prepare(scan: &ScanReport) -> Result<Self, String> {
        let mut sections = Vec::new();
        for section in &scan.sections {
            sections.push((
                section.address,
                read(section.address as usize, section.bytes)?,
            ));
        }
        let locate = |signature: &str, offset: usize| -> Result<usize, String> {
            let pattern = Pattern::parse(&format!("&{signature}"))?;
            let mut found = Vec::new();
            for (base, bytes) in &sections {
                found.extend(
                    pattern
                        .matches(bytes, *base, 2)?
                        .into_iter()
                        .map(|m| m.address as usize),
                );
            }
            if found.len() != 1 {
                return Err("Local login signature is missing or ambiguous".into());
            }
            let address = found[0]
                .checked_add(offset)
                .ok_or("Menu operand overflow")?;
            number(address)?;
            Ok(address)
        };
        let menu = locate("8B480C85C974??8B510885D274??3B05", 16)?;
        let license = locate("895E1C895E14896E1889??????????EB??89??????????68", 0x0b)?;
        let lobby = locate("89412C8B15????????897C2410894230A1", 5)?;
        let select = locate("89412C8B15????????897C2410894230A1", 0x11)?;
        let index = locate("A1????????8B5108406689424C8B4908668B414C50E8", 1)?;
        let yesno = locate("895E1C895E14896E1889??????????EB??89??????????68", 0x47)?;
        Ok(Self {
            pointers: [menu, license, lobby, select, index, yesno],
            code_ranges: scan
                .sections
                .iter()
                .map(|s| (s.address as usize, s.address as usize + s.bytes))
                .collect(),
            steps: AtomicU32::new(0),
        })
    }
    pub fn completed_steps(&self) -> u32 {
        self.steps.load(Ordering::Acquire)
    }
    pub fn done(&self) -> bool {
        self.completed_steps() & 8 != 0
    }
    fn menu_name(&self) -> Result<Option<String>, String> {
        let mut pointer = self.pointers[0];
        for offset in [0, 0, 4] {
            pointer = number(pointer + offset)?;
            if pointer == 0 {
                return Ok(None);
            }
        }
        let bytes = read(pointer + 0x46, 16)?;
        let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
        Ok(Some(
            String::from_utf8_lossy(&bytes[..end]).trim().to_owned(),
        ))
    }
    pub fn step(&self) -> Result<(), String> {
        if self.done() {
            return Ok(());
        }
        let Some(name) = self.menu_name()? else {
            return Ok(());
        };
        let (slot, select, end, bit) = match name.as_str() {
            "menu    ptc8lice" => (1, 0x18, 0x1c, 1),
            "menu    loby2win" => (2, 0x18, 0x1c, 2),
            "menu    dbnamese" => (3, 0x14, 0x1c, 4),
            "menu    ptc6yesn" => (5, 0x14, 0x30, 8),
            _ => return Ok(()),
        };
        if self.completed_steps() & bit != 0 && slot != 3 {
            return Ok(());
        }
        // Only confirm the entry sequence that this navigator selected itself.
        // Other uses of the game's generic confirmation menu are never accepted.
        if (slot == 3 && self.completed_steps() & 2 == 0)
            || (slot == 5 && self.completed_steps() & 6 != 6)
        {
            return Ok(());
        }
        let global = number(self.pointers[slot])?;
        if global == 0 {
            return Ok(());
        }
        let object = number(global)?;
        if object == 0 {
            return Ok(());
        }
        let header = read(object, 0x14)?;
        if header[0x0c] == 0 {
            return Ok(());
        }
        let vtable = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;
        let first = number(vtable)?;
        if !self
            .code_ranges
            .iter()
            .any(|(start, end)| first >= *start && first < *end)
        {
            return Err("Unexpected local menu vtable".into());
        }
        if number(object + end)? != 0 {
            return Ok(());
        }
        // Check all participating fields before writing any of them.
        if number(object + select)? > 16 {
            return Err("Unexpected local menu selection".into());
        }
        let index = if slot == 3 {
            let p = number(self.pointers[4])?;
            if p == 0 {
                return Ok(());
            }
            if number(p)? > 16 {
                return Err("Invalid character slot index".into());
            }
            Some(p)
        } else {
            None
        };
        put(object + select, 0)?;
        if let Some(index) = index {
            put(index, 0)?;
            put(object + 0x18, 0)?;
        }
        put(object + end, 1)?;
        self.steps.fetch_or(bit, Ordering::Release);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn character_selection_retries_after_game_consumes_flag_before_transition() {
        let mut text = [0u8; 0x70];
        text[0x46..0x56].copy_from_slice(b"menu    dbnamese");
        let mut chain = [0u32; 2];
        chain[1] = text.as_ptr() as u32;
        let chain_ref = chain.as_ptr() as u32;
        let global = &chain_ref as *const u32 as u32;
        let mut object = [0u32; 16];
        object[3] = 1;
        object[5] = 3;
        let method = 123456u32;
        object[0] = &method as *const u32 as u32;
        let object_ptr = object.as_mut_ptr() as u32;
        let object_slot = &object_ptr as *const u32 as u32;
        let mut index = 3u32;
        let index_ptr = &mut index as *mut u32 as u32;
        let login = Login {
            pointers: [
                &global as *const u32 as usize,
                0,
                0,
                &object_slot as *const u32 as usize,
                &index_ptr as *const u32 as usize,
                0,
            ],
            code_ranges: vec![(123456, 123457)],
            steps: AtomicU32::new(2),
        };
        login.step().unwrap();
        assert_eq!(object[5], 0);
        assert_eq!(object[7], 1);
        assert_eq!(index, 0);
        object[7] = 0;
        std::hint::black_box(&object);
        login.step().unwrap();
        assert_eq!(object[7], 1);
        assert_eq!(login.completed_steps(), 6);
    }
    #[test]
    fn unknown_menu_never_writes_and_license_transition_is_once_only() {
        let mut text = [0u8; 0x70];
        text[0x46..0x56].copy_from_slice(b"menu    ptc8lice");
        let mut chain = [0u32; 2];
        chain[1] = text.as_ptr() as u32;
        let object_ref = chain.as_ptr() as u32;
        let global_ref = &object_ref as *const u32 as u32;
        let mut object = [0u32; 16];
        object[3] = 1;
        object[6] = 1;
        let method = 123456u32;
        object[0] = &method as *const u32 as u32;
        let object_ptr = object.as_mut_ptr() as u32;
        let object_slot = &object_ptr as *const u32 as u32;
        let operand = &object_slot as *const u32 as usize;
        let login = Login {
            pointers: [&global_ref as *const u32 as usize, operand, 0, 0, 0, 0],
            code_ranges: vec![(123456, 123457)],
            steps: AtomicU32::new(0),
        };
        login.step().unwrap();
        assert_eq!(object[6], 0);
        assert_eq!(object[7], 1);
        assert_eq!(login.completed_steps(), 1);
        object[6] = 1;
        login.step().unwrap();
        assert_eq!(object[6], 1);
        text[0x46] = b'X';
        // The native memory reader observes this through an integer address.
        std::hint::black_box(&text);
        object[7] = 0;
        login.steps.store(0, Ordering::Release);
        login.step().unwrap();
        assert_eq!(object[6], 1);
        assert_eq!(object[7], 0);
        text[0x46..0x56].copy_from_slice(b"menu    ptc6yesn");
        std::hint::black_box(&text);
        login.step().unwrap(); // An unrelated confirmation must not be touched.
        assert_eq!(login.completed_steps(), 0);
    }
}
