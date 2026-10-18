//! Read-only Registry32 client discovery, following the 4.7.9.3 launcher's
//! ClientInfo region paths, installation markers and Steam app-ID ordering.
use crate::profile::Region;
use serde::Serialize;
use std::path::PathBuf;

pub const STEAM_APP_IDS: [&str; 4] = ["23360", "23390", "39250", "39260"];

pub fn region_key(region: Region) -> Result<&'static str, String> {
    match region {
        Region::JP => Ok("SOFTWARE\\PlayOnline"),
        Region::US => Ok("SOFTWARE\\PlayOnlineUS"),
        Region::EU => Ok("SOFTWARE\\PlayOnlineEU"),
        Region::Auto => Err("Auto must be resolved to an installed region".into()),
    }
}

#[derive(Debug, Serialize)]
pub struct ClientInstallation {
    pub region: Region,
    pub play_online_path: Option<PathBuf>,
    pub ffxi_path: Option<PathBuf>,
    pub installation_marker_present: bool,
    pub launcher_exists: bool,
    pub game_dll_exists: bool,
}

impl ClientInstallation {
    /// Windower's InstalledRegions requires an install marker and pol.exe.
    pub fn available(&self) -> bool {
        self.installation_marker_present && self.launcher_exists
    }
}

pub fn select_region(
    clients: &[ClientInstallation],
    requested: Region,
) -> Result<&ClientInstallation, String> {
    // Injector.Launch prefers US for Auto even when JP precedes it in the
    // InstalledRegions enumeration. Otherwise it takes the first region.
    if requested == Region::Auto
        && let Some(client) = clients
            .iter()
            .find(|client| client.available() && client.region == Region::US)
    {
        return Ok(client);
    }
    clients
        .iter()
        .find(|client| {
            client.available() && (requested == Region::Auto || client.region == requested)
        })
        .ok_or_else(|| format!("No installed PlayOnline client for {requested:?}"))
}

#[derive(Serialize)]
pub struct DiscoveryReport {
    pub clients: Vec<ClientInstallation>,
    pub steam_protocol_registered: bool,
    pub steam_app_id: Option<String>,
    pub registry_view: String,
}

#[cfg(windows)]
pub fn discover() -> Result<DiscoveryReport, String> {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, ptr};
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::{
            HKEY, HKEY_CLASSES_ROOT, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY,
            RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegCloseKey, RegGetValueW, RegOpenKeyExW,
        },
    };
    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    fn open(root: HKEY, path: &str) -> Result<Option<Key>, String> {
        let mut key = ptr::null_mut();
        let code = unsafe {
            RegOpenKeyExW(
                root,
                wide(path).as_ptr(),
                0,
                KEY_READ | KEY_WOW64_32KEY,
                &mut key,
            )
        };
        match code {
            ERROR_SUCCESS => Ok(Some(Key(key))),
            ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Ok(None),
            _ => Err(format!("Registry32 {path}: Windows error {code}")),
        }
    }
    fn value(key: &Key, name: &str) -> Result<Option<PathBuf>, String> {
        // Path values only; the bound also permits long Windows paths.
        let mut units = vec![0u16; 32768];
        let mut length = (units.len() * 2) as u32;
        let code = unsafe {
            RegGetValueW(
                key.0,
                ptr::null(),
                wide(name).as_ptr(),
                RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
                ptr::null_mut(),
                units.as_mut_ptr().cast(),
                &mut length,
            )
        };
        match code {
            ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Ok(None),
            ERROR_SUCCESS => {
                if !length.is_multiple_of(2) || length == 0 || length as usize > units.len() * 2 {
                    return Err(format!("Malformed registry path {name}"));
                }
                let units = &units[..length as usize / 2];
                let end = units
                    .iter()
                    .position(|unit| *unit == 0)
                    .ok_or("Unterminated registry path")?;
                Ok(Some(PathBuf::from(OsString::from_wide(&units[..end]))))
            }
            _ => Err(format!("Registry path {name}: Windows error {code}")),
        }
    }
    let mut clients = Vec::new();
    for region in [Region::JP, Region::US, Region::EU] {
        let key = open(
            HKEY_LOCAL_MACHINE,
            &format!("{}\\InstallFolder", region_key(region)?),
        )?;
        let (ffxi_path, play_online_path, installation_marker_present) = if let Some(key) = key {
            let ffxi_path = value(&key, "0001")?;
            let marker = ffxi_path.is_some() || value(&key, "0015")?.is_some();
            (ffxi_path, value(&key, "1000")?, marker)
        } else {
            (None, None, false)
        };
        let launcher_exists = play_online_path
            .as_ref()
            .is_some_and(|path| path.join("pol.exe").is_file());
        let game_dll_exists = ffxi_path
            .as_ref()
            .is_some_and(|path| path.join("FFXiMain.dll").is_file());
        clients.push(ClientInstallation {
            region,
            play_online_path,
            ffxi_path,
            installation_marker_present,
            launcher_exists,
            game_dll_exists,
        });
    }
    let mut steam_app_id = None;
    for app in STEAM_APP_IDS {
        if open(
            HKEY_LOCAL_MACHINE,
            &format!("SOFTWARE\\Valve\\Steam\\Apps\\{app}"),
        )?
        .is_some()
        {
            steam_app_id = Some(app.into());
            break;
        }
    }
    Ok(DiscoveryReport {
        clients,
        steam_app_id,
        steam_protocol_registered: open(HKEY_CLASSES_ROOT, "steam")?.is_some(),
        registry_view: "Registry32".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_preserves_region_order_and_checks_installation_markers() {
        let mut clients = [Region::JP, Region::US, Region::EU].map(|region| ClientInstallation {
            region,
            play_online_path: None,
            ffxi_path: None,
            installation_marker_present: false,
            launcher_exists: true,
            game_dll_exists: false,
        });
        clients[1].installation_marker_present = true;
        clients[2].installation_marker_present = true;
        assert_eq!(
            select_region(&clients, Region::Auto).unwrap().region,
            Region::US
        );
        assert_eq!(
            select_region(&clients, Region::EU).unwrap().region,
            Region::EU
        );
        assert!(select_region(&clients, Region::JP).is_err());
        clients[0].installation_marker_present = true;
        assert_eq!(
            select_region(&clients, Region::Auto).unwrap().region,
            Region::US
        );
        clients[1].launcher_exists = false;
        assert_eq!(
            select_region(&clients, Region::Auto).unwrap().region,
            Region::JP
        );
        clients[1].launcher_exists = true;
        clients[0].launcher_exists = false;
        assert_eq!(
            select_region(&clients, Region::Auto).unwrap().region,
            Region::US
        );
        assert_eq!(region_key(Region::JP).unwrap(), "SOFTWARE\\PlayOnline");
        assert!(region_key(Region::Auto).is_err());
    }
}
