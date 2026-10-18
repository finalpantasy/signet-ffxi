//! Selected process-local FFXI display values; no registry writes.
use crate::protocol::DecodedSettings;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayValues {
    pub width: u32,
    pub height: u32,
    pub ui_width: u32,
    pub ui_height: u32,
    pub mode: u32,
}
impl DisplayValues {
    pub fn from_settings(settings: &DecodedSettings) -> Result<Self, String> {
        if settings.fullscreen
            || [
                settings.width,
                settings.height,
                settings.ui_width,
                settings.ui_height,
            ]
            .iter()
            .any(|v| !(1..=16384).contains(v))
        {
            return Err("Virtual display requires windowed dimensions between 1 and 16384".into());
        }
        Ok(Self {
            width: settings.width as u32,
            height: settings.height as u32,
            ui_width: settings.ui_width as u32,
            ui_height: settings.ui_height as u32,
            mode: if settings.borderless { 3 } else { 1 },
        })
    }
    pub fn setting(&self, name: &[u8]) -> Option<u32> {
        Some(match name {
            b"0001" | b"0003" => self.width,
            b"0002" | b"0004" => self.height,
            b"0034" => self.mode,
            b"0037" => self.ui_width,
            b"0038" => self.ui_height,
            _ => return None,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn four_k_ui_and_window_mode_are_independent_and_only_audited_names_are_overridden() {
        let display = DisplayValues {
            width: 3840,
            height: 2160,
            ui_width: 2560,
            ui_height: 1440,
            mode: 3,
        };
        assert_eq!(display.setting(b"0001"), Some(3840));
        assert_eq!(display.setting(b"0004"), Some(2160));
        assert_eq!(display.setting(b"0034"), Some(3));
        assert_eq!(display.setting(b"0037"), Some(2560));
        assert_eq!(display.setting(b"0038"), Some(1440));
        assert_eq!(display.setting(b"1"), None);
        assert_eq!(display.setting(b"0001\0"), None);
        assert_eq!(display.setting(b"0040"), None);
    }
}
