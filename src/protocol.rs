use crate::profile::{Profile, Region, WindowMode, initialization_command, key_internal_name};
use serde::Serialize;

pub const SETTINGS_MAPPING: &str = "WindowerMMFSettingsHandler";
pub const INJECTION_SIGNAL: &str = "windower.signal-injection";
pub const SETTINGS_BYTES: usize = 42180;

/// Our launcher/runtime always use a private child session. The legacy global
/// names above describe the reference protocol, not our live transport.
pub fn runtime_mapping_names(process_id: u32) -> (String, String) {
    (
        format!("WindowerRust.{process_id}.settings"),
        format!("WindowerRust.{process_id}.signal"),
    )
}

#[derive(Clone, Debug, Serialize)]
pub struct DecodedSettings {
    pub profile_name: String,
    pub branch: String,
    pub launcher_version: String,
    pub hook_version: String,
    pub play_online_path: String,
    pub ffxi_path: String,
    pub windower_path: String,
    pub console_key: String,
    pub region: i32,
    pub width: i32,
    pub height: i32,
    pub ui_width: i32,
    pub ui_height: i32,
    pub x_position: i32,
    pub y_position: i32,
    pub fullscreen: bool,
    pub borderless: bool,
    pub always_enable_gamepad: bool,
    pub allow_win_key: bool,
    pub increase_account_limit: bool,
    pub enable_fast_login: bool,
    pub enable_background_activity: bool,
    pub init_command: String,
    pub init_plugins: String,
    pub init_addons: String,
}

pub fn decode_settings(bytes: &[u8]) -> Result<DecodedSettings, String> {
    if bytes.len() != SETTINGS_BYTES {
        return Err("Incorrect settings payload length".into());
    }
    let mut cursor = Cursor { bytes, position: 0 };
    let result = DecodedSettings {
        profile_name: cursor.wide(256)?,
        branch: cursor.wide(32)?,
        launcher_version: cursor.wide(16)?,
        hook_version: cursor.wide(16)?,
        play_online_path: cursor.wide(4096)?,
        ffxi_path: cursor.wide(4096)?,
        windower_path: cursor.wide(4096)?,
        console_key: cursor.wide(16)?,
        region: cursor.integer()?,
        width: cursor.integer()?,
        height: cursor.integer()?,
        ui_width: cursor.integer()?,
        ui_height: cursor.integer()?,
        x_position: cursor.integer()?,
        y_position: cursor.integer()?,
        fullscreen: cursor.boolean()?,
        borderless: cursor.boolean()?,
        always_enable_gamepad: cursor.boolean()?,
        allow_win_key: cursor.boolean()?,
        increase_account_limit: cursor.boolean()?,
        enable_fast_login: cursor.boolean()?,
        enable_background_activity: cursor.boolean()?,
        init_command: cursor.wide(4096)?,
        init_plugins: cursor.wide(256)?,
        init_addons: cursor.wide(4096)?,
    };
    if !matches!(result.region, 0..=2) {
        return Err("Invalid/unresolved game region".into());
    }
    if [
        result.width,
        result.height,
        result.ui_width,
        result.ui_height,
    ]
    .iter()
    .any(|n| *n <= 0)
    {
        return Err("Invalid game dimensions".into());
    }
    if cursor.position != SETTINGS_BYTES {
        return Err("Incorrect decoded payload length".into());
    }
    Ok(result)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl Cursor<'_> {
    fn take(&mut self, length: usize) -> Result<&[u8], String> {
        let end = self
            .position
            .checked_add(length)
            .ok_or("Settings size overflow")?;
        let result = self
            .bytes
            .get(self.position..end)
            .ok_or("Truncated settings payload")?;
        self.position = end;
        Ok(result)
    }
    fn wide(&mut self, capacity: usize) -> Result<String, String> {
        if !self.position.is_multiple_of(2) {
            self.take(1)?;
        }
        let bytes = self.take(capacity * 2)?;
        let units = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        let end = units
            .iter()
            .position(|unit| *unit == 0)
            .ok_or("Unterminated protocol string")?;
        String::from_utf16(&units[..end]).map_err(|_| "Invalid UTF-16 protocol string".into())
    }
    fn integer(&mut self) -> Result<i32, String> {
        let bytes = self.take(4)?;
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
    fn boolean(&mut self) -> Result<bool, String> {
        match self.take(1)?[0] {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err("Invalid protocol boolean".into()),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct LaunchContext {
    pub branch: String,
    pub launcher_version: String,
    pub hook_version: String,
    pub play_online_path: String,
    pub ffxi_path: String,
    pub windower_path: String,
    pub region: Region,
    pub display_width: i32,
    pub display_height: i32,
    pub plugins: Vec<String>,
    pub addons: Vec<String>,
}

pub fn format_directory(path: &str) -> Result<String, String> {
    if path.is_empty() || path.contains('\0') {
        return Err("Invalid empty/NUL directory".into());
    }
    let path = path.replace('/', "\\");
    let mut chars = path.chars();
    let first = chars.next().ok_or("Empty directory")?;
    let mut result = format!("{}{rest}", first.to_uppercase(), rest = chars.as_str());
    if !result.ends_with('\\') {
        result.push('\\');
    }
    Ok(result)
}

// Explicit byte serialization avoids relying on Rust struct layout, bool ABI,
// alignment or host pointer size. The reference is Injector.Settings, Pack=4.
pub fn encode_settings(p: &Profile, c: &LaunchContext) -> Result<Vec<u8>, String> {
    if c.region == Region::Auto {
        return Err("Resolve the installed region first".into());
    }
    let (width, height) = if p.width == 0 || p.height == 0 {
        (c.display_width, c.display_height)
    } else {
        (p.width, p.height)
    };
    if width <= 0 || height <= 0 {
        return Err("Display dimensions must be positive".into());
    }
    let mut ui_width = dotnet_float_to_int(width as f32 / p.ui_scale);
    let mut ui_height = dotnet_float_to_int(height as f32 / p.ui_scale);
    if ui_width <= 0 || ui_height <= 0 {
        ui_width = width;
        ui_height = height;
    }
    let mut b = Vec::with_capacity(SETTINGS_BYTES);
    wide(&mut b, &p.name, 256)?;
    wide(&mut b, &c.branch, 32)?;
    wide(&mut b, &c.launcher_version, 16)?;
    wide(&mut b, &c.hook_version, 16)?;
    wide(&mut b, &format_directory(&c.play_online_path)?, 4096)?;
    wide(&mut b, &format_directory(&c.ffxi_path)?, 4096)?;
    wide(&mut b, &format_directory(&c.windower_path)?, 4096)?;
    wide(&mut b, key_internal_name(&p.console_key), 16)?;
    for n in [
        c.region.wire_value(),
        width,
        height,
        ui_width,
        ui_height,
        0,
        0,
    ] {
        b.extend_from_slice(&n.to_le_bytes());
    }
    b.extend_from_slice(&[
        u8::from(p.window_mode == WindowMode::Fullscreen),
        u8::from(p.window_mode == WindowMode::BorderlessWindow),
        u8::from(p.always_enable_gamepad),
        u8::from(p.allow_win_key),
        u8::from(p.increase_account_limit),
        u8::from(p.enable_fast_login),
        u8::from(p.enable_background_activity),
    ]);
    wide(
        &mut b,
        &initialization_command(p.init_command.as_deref()),
        4096,
    )?;
    wide(&mut b, &c.plugins.join(";"), 256)?;
    wide(&mut b, &c.addons.join(";"), 4096)?;
    while !b.len().is_multiple_of(4) {
        b.push(0);
    }
    if b.len() != SETTINGS_BYTES {
        return Err(format!("Unexpected settings length {}", b.len()));
    }
    Ok(b)
}

fn wide(bytes: &mut Vec<u8>, value: &str, capacity: usize) -> Result<(), String> {
    if value.contains('\0') {
        return Err("NUL inside protocol string".into());
    }
    let units: Vec<u16> = value.encode_utf16().collect();
    // Reject instead of silently truncating paths or executable commands.
    if units.len() >= capacity {
        return Err(format!(
            "Protocol string exceeds {} UTF-16 units",
            capacity - 1
        ));
    }
    if !bytes.len().is_multiple_of(2) {
        bytes.push(0);
    }
    for unit in &units {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes.resize(bytes.len() + (capacity - units.len()) * 2, 0);
    Ok(())
}

fn dotnet_float_to_int(value: f32) -> i32 {
    if !value.is_finite() || !(-2147483648.0..2147483648.0).contains(&value) {
        i32::MIN
    } else {
        value as i32
    }
}
