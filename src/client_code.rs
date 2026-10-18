//! Read-only identification of loaded FFXI executable code. Reports contain
//! hashes and signature locations, never captured bytes or game data.
use crate::pattern::{FFXI_SIGNATURES, Pattern};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, PartialEq, Eq)]
pub struct CodeSection {
    pub name: String,
    pub rva: u32,
    pub size: u32,
}

/// Parse an in-memory i386 PE header. Raw file offsets/sizes are deliberately
/// ignored: the installed client has an empty on-disk .text section.
pub fn executable_sections(headers: &[u8], mapped_size: u32) -> Result<Vec<CodeSection>, String> {
    fn word(bytes: &[u8], offset: usize) -> Result<u16, String> {
        Ok(u16::from_le_bytes(
            bytes
                .get(offset..offset + 2)
                .ok_or("Truncated PE header")?
                .try_into()
                .map_err(|_| "Invalid word")?,
        ))
    }
    fn dword(bytes: &[u8], offset: usize) -> Result<u32, String> {
        Ok(u32::from_le_bytes(
            bytes
                .get(offset..offset + 4)
                .ok_or("Truncated PE header")?
                .try_into()
                .map_err(|_| "Invalid dword")?,
        ))
    }
    if headers.get(..2) != Some(b"MZ") {
        return Err("Missing DOS signature".into());
    }
    let pe = dword(headers, 0x3c)? as usize;
    if pe > 0x8000 || headers.get(pe..pe + 4) != Some(b"PE\0\0") {
        return Err("Invalid PE header location/signature".into());
    }
    if word(headers, pe + 4)? != 0x14c || word(headers, pe + 24)? != 0x10b {
        return Err("Expected an i386 PE32 client".into());
    }
    let count = word(headers, pe + 6)? as usize;
    let optional_size = word(headers, pe + 20)? as usize;
    if count == 0 || count > 96 || optional_size < 96 {
        return Err("Invalid PE section table".into());
    }
    let image_size = dword(headers, pe + 24 + 56)?;
    if image_size == 0 || image_size > mapped_size || image_size > 128 * 1024 * 1024 {
        return Err("Invalid or excessive mapped image size".into());
    }
    let table = pe + 24 + optional_size;
    let mut result: Vec<CodeSection> = Vec::new();
    for index in 0..count {
        let entry = table + index * 40;
        let bytes = headers
            .get(entry..entry + 40)
            .ok_or("Truncated PE section table")?;
        if dword(bytes, 36)? & 0x2000_0000 == 0 {
            continue;
        }
        let size = dword(bytes, 8)?;
        let rva = dword(bytes, 12)?;
        let end = rva.checked_add(size).ok_or("Section address overflow")?;
        if size == 0 || rva == 0 || end > image_size {
            return Err("Executable section escapes mapped image".into());
        }
        if result
            .iter()
            .any(|other| rva < other.rva + other.size && other.rva < end)
        {
            return Err("Overlapping executable sections".into());
        }
        let name_end = bytes[..8].iter().position(|byte| *byte == 0).unwrap_or(8);
        result.push(CodeSection {
            name: String::from_utf8_lossy(&bytes[..name_end]).into(),
            rva,
            size,
        });
    }
    if result.is_empty() {
        return Err("No executable sections".into());
    }
    Ok(result)
}

pub struct CapturedCode {
    pub name: String,
    pub address: u32,
    pub bytes: Vec<u8>,
}
pub struct LoadedCodeCapture {
    pub report: ScanReport,
    pub code: Vec<CapturedCode>,
}
impl LoadedCodeCapture {
    pub fn read(directory: &std::path::Path) -> Result<Self, String> {
        let root = directory
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let manifest = root.join("manifest.json");
        if std::fs::metadata(&manifest)
            .map_err(|error| error.to_string())?
            .len()
            > 65536
        {
            return Err("Code capture manifest exceeds limit".into());
        }
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(manifest).map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
        if manifest["schema_version"] != 1 {
            return Err("Unsupported capture schema".into());
        }
        let original: ScanReport =
            serde_json::from_value(manifest["scan"].clone()).map_err(|error| error.to_string())?;
        let files = manifest["files"]
            .as_array()
            .ok_or("Missing code capture files")?;
        if files.is_empty() || files.len() > 96 {
            return Err("Invalid code capture section count".into());
        }
        if files.len() != original.sections.len() {
            return Err("Capture files do not cover the original executable section report".into());
        }
        let (mut code, mut total) = (Vec::new(), 0u64);
        for (index, file) in files.iter().enumerate() {
            let expected = format!("code-{index:03}.bin");
            if file["file"].as_str() != Some(&expected) {
                return Err("Invalid code capture filename".into());
            }
            let path = root
                .join(expected)
                .canonicalize()
                .map_err(|error| error.to_string())?;
            if !path.starts_with(&root) {
                return Err("Code capture escapes selected directory".into());
            }
            let size = std::fs::metadata(&path)
                .map_err(|error| error.to_string())?
                .len();
            total = total.checked_add(size).ok_or("Code size overflow")?;
            if size == 0 || file["bytes"].as_u64() != Some(size) || total > 128 * 1024 * 1024 {
                return Err("Code capture size mismatch or limit exceeded".into());
            }
            let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
            if file["sha256"].as_str() != Some(&format!("{:x}", Sha256::digest(&bytes))) {
                return Err("Code capture digest mismatch".into());
            }
            let address = u32::try_from(file["address"].as_u64().ok_or("Invalid code address")?)
                .map_err(|_| "Code address exceeds i686")?;
            let expected = &original.sections[index];
            if address != expected.address
                || bytes.len() != expected.bytes
                || file["section"].as_str() != Some(&expected.name)
                || file["sha256"].as_str() != Some(&expected.sha256)
            {
                return Err("Capture metadata disagrees with original section report".into());
            }
            address
                .checked_add(u32::try_from(bytes.len()).map_err(|_| "Code size exceeds i686")?)
                .ok_or("Code address overflow")?;
            code.push(CapturedCode {
                name: file["section"]
                    .as_str()
                    .ok_or("Invalid section name")?
                    .into(),
                address,
                bytes,
            });
        }
        let mut report = scan_code(original.process_id, original.module_base, &code)?;
        report.complete_executable_coverage &= original.complete_executable_coverage;
        Ok(Self { report, code })
    }
    /// Explicit local export of executable sections only. Ordinary scan-client
    /// reports still contain no raw code. No data sections or heaps are saved.
    pub fn write_new(&self, directory: &std::path::Path) -> Result<(), String> {
        use std::io::Write;
        std::fs::create_dir(directory).map_err(|error| error.to_string())?;
        let mut files = Vec::new();
        for (index, section) in self.code.iter().enumerate() {
            let name = format!("code-{index:03}.bin");
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join(&name))
                .and_then(|mut file| file.write_all(&section.bytes))
                .map_err(|error| error.to_string())?;
            files.push(serde_json::json!({"file":name,"section":section.name,"address":section.address,
                "bytes":section.bytes.len(),"sha256":format!("{:x}", Sha256::digest(&section.bytes))}));
        }
        let manifest = serde_json::to_vec_pretty(&serde_json::json!({"schema_version":1,"scan":self.report,"files":files,
            "scope":"PE executable sections only; no heap or nonexecutable data sections","atomic_snapshot":false}))
            .map_err(|error| error.to_string())?;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("manifest.json"))
            .and_then(|mut file| file.write_all(&manifest))
            .map_err(|error| error.to_string())
    }
}

#[derive(Serialize, Deserialize)]
pub struct SignatureResult {
    pub name: String,
    pub status: String,
    pub locations: Vec<(u32, u32)>,
}
#[derive(Serialize, Deserialize)]
pub struct SectionReport {
    pub name: String,
    pub address: u32,
    pub bytes: usize,
    pub sha256: String,
}
#[derive(Serialize, Deserialize)]
pub struct ScanReport {
    pub process_id: u32,
    pub module: String,
    pub module_base: u32,
    pub sections: Vec<SectionReport>,
    pub signatures: Vec<SignatureResult>,
    #[serde(default)]
    pub patched_candidates: Vec<PatchedCandidate>,
    pub complete_executable_coverage: bool,
    pub game_calls_validated: bool,
    pub limits: Vec<String>,
}
#[derive(Serialize, Deserialize)]
pub struct PatchedCandidate {
    pub name: String,
    pub entry: u32,
    pub jump_destination: u32,
    pub reference_prefix_bytes: usize,
    pub tail_match_count: usize,
    pub callable: bool,
}

fn patched_candidates(code: &[CapturedCode]) -> Vec<PatchedCandidate> {
    // Only diagnostic candidates. Original prologues were overwritten in the
    // observed Windower session. Tail matches never enable a game binding.
    const TAILS: &[(&str, usize, &[u8])] = &[
        (
            "add_to_chat",
            6,
            &[0x53, 0x55, 0x56, 0x57, 0x8b, 0xe9, 0x6a, 0x08, 0x6a, 0x00],
        ),
        (
            "input_command",
            6,
            &[0x8b, 0x8c, 0x24, 0x08, 0x04, 0, 0, 0x55, 0x56, 0x57],
        ),
        (
            "decode_packet",
            10,
            &[
                0x53, 0x55, 0x8b, 0xac, 0x24, 0xc8, 0x05, 0, 0, 0x56, 0x3b, 0xe8, 0x57,
            ],
        ),
        (
            "encode_packet",
            5,
            &[
                0x53, 0x33, 0xc0, 0x55, 0x8b, 0x6c, 0x24, 0x14, 0x49, 0x56, 0x57, 0x8d, 0x14, 0xcd,
                0, 0, 0, 0, 0x33, 0xff, 0x85, 0xed, 0x89, 0x54, 0x24, 0x10,
            ],
        ),
    ];
    let mut result = Vec::new();
    for (name, prefix, tail) in TAILS {
        let mut candidates = Vec::new();
        let mut count = 0;
        for section in code {
            for (offset, bytes) in section.bytes.windows(tail.len()).enumerate() {
                if bytes != *tail {
                    continue;
                }
                count += 1;
                let Some(start) = offset.checked_sub(*prefix) else {
                    continue;
                };
                let head = &section.bytes[start..offset];
                if head[0] != 0xe9 || !head[5..].iter().all(|byte| matches!(byte, 0x90 | 0xcc)) {
                    continue;
                }
                let Some(entry) = section.address.checked_add(start as u32) else {
                    continue;
                };
                let displacement =
                    u32::from_le_bytes(head[1..5].try_into().expect("Five-byte jump checked"));
                candidates.push(PatchedCandidate {
                    name: (*name).into(),
                    entry,
                    jump_destination: entry.wrapping_add(5).wrapping_add(displacement),
                    reference_prefix_bytes: *prefix,
                    tail_match_count: 0,
                    callable: false,
                });
            }
        }
        for mut candidate in candidates {
            candidate.tail_match_count = count;
            result.push(candidate);
        }
    }
    result
}

pub fn scan_code(
    process_id: u32,
    module_base: u32,
    code: &[CapturedCode],
) -> Result<ScanReport, String> {
    let mut signatures = Vec::new();
    for (name, source) in FFXI_SIGNATURES {
        let pattern = Pattern::parse(source)?;
        let mut locations = Vec::new();
        for section in code {
            locations.extend(
                pattern
                    .matches(&section.bytes, section.address, 2)?
                    .into_iter()
                    .map(|found| (found.address, found.resolved)),
            );
        }
        let status = match locations.len() {
            0 => "missing",
            1 => "unique",
            _ => "ambiguous",
        };
        signatures.push(SignatureResult {
            name: (*name).into(),
            status: status.into(),
            locations,
        });
    }
    Ok(ScanReport {
        process_id,
        module: "FFXiMain.dll".into(),
        module_base,
        sections: code
            .iter()
            .map(|section| SectionReport {
                name: section.name.clone(),
                address: section.address,
                bytes: section.bytes.len(),
                sha256: format!("{:x}", Sha256::digest(&section.bytes)),
            })
            .collect(),
        signatures,
        patched_candidates: patched_candidates(code),
        complete_executable_coverage: !code.is_empty(),
        game_calls_validated: false,
        limits: vec![
            "Signature matches alone do not establish a calling convention or safe game call"
                .into(),
            "Running Windower may have patched function prologues; missing matches are not guessed"
                .into(),
            "Code is read without pausing the client; this is not an atomic snapshot".into(),
        ],
    })
}

#[cfg(all(windows, target_pointer_width = "32"))]
pub fn scan_loaded_client(process_id: u32) -> Result<ScanReport, String> {
    Ok(capture_loaded_client(process_id)?.report)
}

#[cfg(all(windows, target_pointer_width = "32"))]
pub fn capture_loaded_client(process_id: u32) -> Result<LoadedCodeCapture, String> {
    use std::{
        ffi::c_void,
        mem::{size_of, zeroed},
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::{
                Debug::ReadProcessMemory,
                ToolHelp::{
                    CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW,
                    TH32CS_SNAPMODULE,
                },
            },
            Memory::{
                MEM_COMMIT, MEM_IMAGE, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE_READ,
                PAGE_EXECUTE_READWRITE, PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, VirtualQueryEx,
            },
            Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ},
        },
    };
    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    fn read(process: HANDLE, address: u32, size: usize) -> Result<Vec<u8>, String> {
        let mut bytes = vec![0; size];
        let mut copied = 0;
        if unsafe {
            ReadProcessMemory(
                process,
                address as *const c_void,
                bytes.as_mut_ptr().cast(),
                size,
                &mut copied,
            )
        } == 0
            || copied != size
        {
            return Err(format!(
                "Cannot read selected code at {address:#010x}: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(bytes)
    }
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE, process_id) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let snapshot = Handle(snapshot);
    let mut entry: MODULEENTRY32W = unsafe { zeroed() };
    entry.dwSize = size_of::<MODULEENTRY32W>() as u32;
    let mut found = unsafe { Module32FirstW(snapshot.0, &mut entry) } != 0;
    while found {
        let end = entry
            .szModule
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(entry.szModule.len());
        if String::from_utf16_lossy(&entry.szModule[..end]).eq_ignore_ascii_case("FFXiMain.dll") {
            break;
        }
        found = unsafe { Module32NextW(snapshot.0, &mut entry) } != 0;
    }
    if !found {
        return Err("FFXiMain.dll is not loaded in the selected i686 process".into());
    }
    let base = entry.modBaseAddr as u32;
    let process =
        unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, process_id) };
    if process.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let process = Handle(process);
    // Read only the mapped PE headers, then only complete executable sections.
    let first = read(process.0, base, 4096)?;
    let pe = u32::from_le_bytes(
        first[0x3c..0x40]
            .try_into()
            .map_err(|_| "Invalid DOS header")?,
    ) as usize;
    if pe > 0x8000 {
        return Err("Excessive PE header offset".into());
    }
    let header_size = (pe + 24 + 4096).min(entry.modBaseSize as usize);
    let headers = read(process.0, base, header_size)?;
    let sections = executable_sections(&headers, entry.modBaseSize)?;
    let mut code = Vec::new();
    for section in sections {
        let start = base
            .checked_add(section.rva)
            .ok_or("Module address overflow")?;
        let end = start
            .checked_add(section.size)
            .ok_or("Section address overflow")?;
        let mut cursor = start;
        while cursor < end {
            let mut info: MEMORY_BASIC_INFORMATION = unsafe { zeroed() };
            if unsafe {
                VirtualQueryEx(
                    process.0,
                    cursor as *const c_void,
                    &mut info,
                    size_of::<MEMORY_BASIC_INFORMATION>(),
                )
            } != size_of::<MEMORY_BASIC_INFORMATION>()
            {
                return Err("Cannot query executable section mapping".into());
            }
            let allowed = matches!(
                info.Protect & 0xff,
                PAGE_EXECUTE_READ | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY
            );
            if info.State != MEM_COMMIT
                || info.Type != MEM_IMAGE
                || info.AllocationBase as u32 != base
                || info.Protect & PAGE_GUARD != 0
                || !allowed
            {
                return Err(format!(
                    "Executable section {} is not fully unpacked/readable; no partial scan accepted",
                    section.name
                ));
            }
            let next = (info.BaseAddress as usize)
                .checked_add(info.RegionSize)
                .ok_or("Memory region overflow")?
                .min(end as usize);
            if next <= cursor as usize {
                return Err("Invalid memory region boundary".into());
            }
            cursor = next as u32;
        }
        code.push(CapturedCode {
            name: section.name,
            address: start,
            bytes: read(process.0, start, section.size as usize)?,
        });
    }
    Ok(LoadedCodeCapture {
        report: scan_code(process_id, base, &code)?,
        code,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patched_entry_candidates_require_jumps_and_never_enable_calls() {
        let bytes = vec![
            0xe9, 0xfb, 0x0f, 0, 0, 0xcc, 0x8b, 0x8c, 0x24, 8, 4, 0, 0, 0x55, 0x56, 0x57,
        ];
        let mut code = vec![CapturedCode {
            name: ".text".into(),
            address: 0x1000,
            bytes,
        }];
        let found = patched_candidates(&code);
        assert_eq!(found.len(), 1);
        assert_eq!(
            (
                found[0].entry,
                found[0].jump_destination,
                found[0].tail_match_count
            ),
            (0x1000, 0x2000, 1)
        );
        assert!(!found[0].callable);
        code.push(CapturedCode {
            name: "other".into(),
            address: 0x3000,
            bytes: code[0].bytes.clone(),
        });
        assert!(
            patched_candidates(&code)
                .iter()
                .all(|entry| entry.tail_match_count == 2 && !entry.callable)
        );
        code.truncate(1);
        code[0].bytes[0] = 0x90;
        assert!(patched_candidates(&code).is_empty());
    }
    #[test]
    fn code_capture_round_trip_checks_hashes_and_never_overwrites() {
        let directory = std::env::temp_dir().join(format!(
            "windower-code-capture-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let code = vec![CapturedCode {
            name: ".text".into(),
            address: 0x1000,
            bytes: vec![0x90, 0xc3],
        }];
        let capture = LoadedCodeCapture {
            report: scan_code(1, 0, &code).unwrap(),
            code,
        };
        capture.write_new(&directory).unwrap();
        assert!(capture.write_new(&directory).is_err());
        let restored = LoadedCodeCapture::read(&directory).unwrap();
        assert_eq!(restored.code[0].bytes, vec![0x90, 0xc3]);
        std::fs::write(directory.join("code-000.bin"), [0x90, 0x90]).unwrap();
        assert!(
            LoadedCodeCapture::read(&directory)
                .err()
                .unwrap()
                .contains("digest")
        );
        std::fs::remove_file(directory.join("code-000.bin")).unwrap();
        std::fs::remove_file(directory.join("manifest.json")).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
    fn header() -> Vec<u8> {
        let mut bytes = vec![0; 512];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&0x14cu16.to_le_bytes());
        bytes[0x86..0x88].copy_from_slice(&1u16.to_le_bytes());
        bytes[0x94..0x96].copy_from_slice(&224u16.to_le_bytes());
        bytes[0x98..0x9a].copy_from_slice(&0x10bu16.to_le_bytes());
        bytes[0xd0..0xd4].copy_from_slice(&0x3000u32.to_le_bytes());
        let table = 0x178;
        bytes[table..table + 5].copy_from_slice(b".text");
        bytes[table + 8..table + 12].copy_from_slice(&0x1000u32.to_le_bytes());
        bytes[table + 12..table + 16].copy_from_slice(&0x1000u32.to_le_bytes());
        bytes[table + 36..table + 40].copy_from_slice(&0x6000_0020u32.to_le_bytes());
        bytes
    }
    #[test]
    fn mapped_virtual_code_does_not_require_on_disk_bytes() {
        assert_eq!(
            executable_sections(&header(), 0x3000).unwrap(),
            vec![CodeSection {
                name: ".text".into(),
                rva: 0x1000,
                size: 0x1000
            }]
        );
    }
    #[test]
    fn rejects_truncation_wrong_architecture_and_section_escape() {
        let bytes = header();
        for length in [0, 2, 64, 0x90, 0x190] {
            assert!(executable_sections(&bytes[..length], 0x3000).is_err());
        }
        let mut bytes = header();
        bytes[0x84] = 0x64;
        assert!(executable_sections(&bytes, 0x3000).is_err());
        let mut bytes = header();
        bytes[0x184..0x188].copy_from_slice(&0x2800u32.to_le_bytes());
        assert!(executable_sections(&bytes, 0x3000).is_err());
    }
    #[test]
    fn uniqueness_is_checked_across_all_captured_sections() {
        let bytes = [0x56, 0x8b, 0xf1, 0x8b, 0x86, 0x50, 0x0d, 0, 0].to_vec();
        let code = vec![
            CapturedCode {
                name: "one".into(),
                address: 0x1000,
                bytes: bytes.clone(),
            },
            CapturedCode {
                name: "two".into(),
                address: 0x2000,
                bytes,
            },
        ];
        let report = scan_code(1, 0, &code).unwrap();
        let found = report
            .signatures
            .iter()
            .find(|s| s.name == "draw_scene")
            .unwrap();
        assert_eq!(found.status, "ambiguous");
        assert_eq!(found.locations.len(), 2);
        assert!(!report.game_calls_validated);
        assert!(!scan_code(1, 0, &[]).unwrap().complete_executable_coverage);
    }

    #[cfg(all(windows, target_pointer_width = "32"))]
    #[test]
    fn reads_only_own_disposable_fixture_image() {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        use windows_sys::Win32::{
            Foundation::FreeLibrary,
            System::{
                LibraryLoader::{LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW},
                SystemInformation::GetSystemDirectoryW,
                Threading::GetCurrentProcessId,
            },
        };
        let mut system = [0u16; 512];
        let length =
            unsafe { GetSystemDirectoryW(system.as_mut_ptr(), system.len() as u32) } as usize;
        assert!(length > 0 && length < system.len());
        let source = std::path::PathBuf::from(std::ffi::OsString::from_wide(&system[..length]))
            .join("version.dll");
        let directory = std::env::temp_dir().join(format!(
            "windower-code-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let fixture = directory.join("FFXiMain.dll");
        std::fs::copy(source, &fixture).unwrap();
        let wide = fixture
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let module = unsafe {
            LoadLibraryExW(
                wide.as_ptr(),
                std::ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        };
        assert!(!module.is_null());
        let report = scan_loaded_client(unsafe { GetCurrentProcessId() });
        unsafe {
            FreeLibrary(module);
        }
        std::fs::remove_file(&fixture).unwrap();
        std::fs::remove_dir(&directory).unwrap();
        let report = report.unwrap();
        assert!(report.complete_executable_coverage);
        assert!(!report.sections.is_empty());
        assert!(
            report
                .sections
                .iter()
                .all(|section| section.bytes > 0 && section.sha256.len() == 64)
        );
        assert!(!report.game_calls_validated);
    }
}
