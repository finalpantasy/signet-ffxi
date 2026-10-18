use roxmltree::{Document, Node};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Region {
    Auto,
    JP,
    US,
    EU,
}

impl Region {
    pub fn wire_value(self) -> i32 {
        match self {
            Self::Auto => -1,
            Self::JP => 0,
            Self::US => 1,
            Self::EU => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum WindowMode {
    BorderlessWindow,
    Window,
    Fullscreen,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Profile {
    pub name: String,
    pub region: Region,
    pub console_key: String,
    pub init_command: Option<String>,
    pub window_mode: WindowMode,
    pub width: i32,
    pub height: i32,
    pub mip_map_level: i32,
    #[serde(rename = "UIScale")]
    pub ui_scale: f32,
    pub gamma: f32,
    pub hardware_mouse: bool,
    pub super_sampling: f32,
    pub bump_mapping: bool,
    pub sound: bool,
    pub always_play_sound: bool,
    pub always_enable_gamepad: bool,
    pub allow_win_key: bool,
    pub increase_account_limit: bool,
    pub enable_fast_login: bool,
    pub enable_background_activity: bool,
    pub executable: String,
    pub args: String,
    pub update: bool,
}

impl Profile {
    // The original constructor has UIScale=0; an XML profile defaults it to 1.
    pub fn anonymous() -> Self {
        Self {
            name: String::new(),
            region: Region::Auto,
            console_key: "Insert".into(),
            init_command: None,
            window_mode: WindowMode::BorderlessWindow,
            width: 0,
            height: 0,
            mip_map_level: 0,
            ui_scale: 0.0,
            gamma: 0.0,
            hardware_mouse: true,
            super_sampling: 1.0,
            bump_mapping: false,
            sound: true,
            always_play_sound: true,
            always_enable_gamepad: true,
            allow_win_key: false,
            increase_account_limit: false,
            enable_fast_login: false,
            enable_background_activity: false,
            executable: "pol.exe".into(),
            args: String::new(),
            update: false,
        }
    }

    fn from_xml(node: Node<'_, '_>) -> Self {
        let mut p = Self::anonymous();
        p.name = node.attribute("name").unwrap_or("").trim().into();
        p.region = match field(node, "region")
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "jp" => Region::JP,
            "us" => Region::US,
            "eu" => Region::EU,
            _ => Region::Auto,
        };
        p.console_key = canonical_key(field(node, "consolekey").unwrap_or("Insert"));
        p.init_command = field(node, "initcommand").map(str::to_owned);
        p.window_mode = match field(node, "windowmode")
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "window" => WindowMode::Window,
            "fullscreen" => WindowMode::Fullscreen,
            _ => WindowMode::BorderlessWindow,
        };
        p.width = integer(node, "width", 0);
        p.height = integer(node, "height", 0);
        p.mip_map_level = integer(node, "mipmaplevel", 0);
        p.ui_scale = real(node, "uiscale", 1.0);
        p.gamma = real(node, "gamma", 0.0);
        let sampling = real(node, "supersampling", 1.0);
        p.super_sampling = if sampling <= 0.0 { 1.0 } else { sampling };
        p.hardware_mouse = boolean(node, "hardwaremouse", true);
        p.bump_mapping = boolean(node, "bumpmapping", false);
        p.sound = boolean(node, "sound", true);
        p.always_play_sound = boolean(node, "alwaysplaysound", true);
        p.always_enable_gamepad = boolean(node, "alwaysenablegamepad", true);
        p.allow_win_key = boolean(node, "allowwinkey", false);
        p.increase_account_limit = boolean(node, "increaseaccountlimit", false);
        p.enable_fast_login = boolean(node, "enablefastlogin", false);
        p.enable_background_activity = boolean(node, "preventbackgroundthrottling", false);
        p.executable = field(node, "executable").unwrap_or("pol.exe").into();
        p.args = field(node, "args").unwrap_or("").into();
        p
    }
}

#[derive(Debug, Serialize)]
pub struct Settings {
    pub profiles: Vec<Profile>,
    pub addons: Vec<String>,
    pub plugins: Vec<String>,
}

impl Settings {
    pub fn parse(xml: &str) -> Result<Self, String> {
        let doc = Document::parse(xml).map_err(|e| e.to_string())?;
        let root = doc.root_element();
        let mut profiles = Vec::<Profile>::new();
        for node in root.children().filter(|n| n.has_tag_name("profile")) {
            // Original duplicate check happens before the Name setter trims it.
            let raw_name = node.attribute("name").unwrap_or("");
            if profiles.iter().any(|p| p.name == raw_name) {
                return Err(format!("Profile already exists: {raw_name}"));
            }
            profiles.push(Profile::from_xml(node));
        }
        if profiles.is_empty() {
            profiles.push(Profile::anonymous());
        }
        let autoload = root.children().find(|n| n.has_tag_name("autoload"));
        let names = |tag: &str| {
            let mut result = Vec::new();
            if let Some(node) = autoload {
                for child in node.children().filter(|n| n.has_tag_name(tag)) {
                    let value = child.text().unwrap_or("");
                    if !value.trim().is_empty() && !result.iter().any(|s| s == value) {
                        result.push(value.to_owned());
                    }
                }
            }
            result
        };
        Ok(Self {
            profiles,
            addons: names("addon"),
            plugins: names("plugin"),
        })
    }

    // CurrentCultureIgnoreCase differs from this for some non-ASCII locales.
    pub fn select(&self, name: &str) -> Profile {
        let name = name.trim();
        self.profiles
            .iter()
            .find(|p| p.name.to_lowercase() == name.to_lowercase())
            .or_else(|| self.profiles.iter().find(|p| p.name.is_empty()))
            .cloned()
            .unwrap_or_else(Profile::anonymous)
    }
}

fn field<'a>(node: Node<'a, 'a>, name: &str) -> Option<&'a str> {
    node.children()
        .find(|n| n.has_tag_name(name))
        .map(|n| n.text().unwrap_or(""))
}
fn integer(node: Node<'_, '_>, name: &str, default: i32) -> i32 {
    field(node, name)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(default)
}
fn real(node: Node<'_, '_>, name: &str, default: f32) -> f32 {
    // Covers normal invariant decimal/exponent forms and thousands separators.
    field(node, name)
        .and_then(|s| s.trim().replace(',', "").parse().ok())
        .unwrap_or(default)
}
fn boolean(node: Node<'_, '_>, name: &str, default: bool) -> bool {
    match field(node, name)
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("true") => true,
        Some("false") => false,
        _ => default,
    }
}

pub fn canonical_key(value: &str) -> String {
    include_str!("keys.tsv")
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .find(|(name, _)| name.eq_ignore_ascii_case(value) && *name != "None")
        .map_or_else(|| "Insert".into(), |(name, _)| name.into())
}

pub fn key_internal_name(value: &str) -> &str {
    include_str!("keys.tsv")
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .find(|(name, _)| *name == value)
        .map_or("insert", |(_, internal)| internal)
}

pub fn initialization_command(value: Option<&str>) -> String {
    value
        .unwrap_or("")
        .split(';')
        .map(|part| {
            let trimmed = part.trim();
            if trimmed.starts_with("load ") || trimmed.starts_with("exec ") {
                format!("echo Command not supported in initialization: {part}")
            } else {
                part.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(";")
}
