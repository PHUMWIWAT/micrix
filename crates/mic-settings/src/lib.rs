mod placement;

pub use placement::{ToastRect, WorkArea, toast_rect, work_area_at};

use std::str::FromStr;

use global_hotkey::hotkey::HotKey;
use mic_core::DEFAULT_DEVICE_ID;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const SCHEMA_VERSION: u32 = 1;

pub const ACTIONS: &[&str] = &[
    "toggle_mute",
    "force_mute",
    "force_unmute",
    "hold_to_mute",
    "open_settings",
];

pub const TOAST_POSITIONS: &[&str] = &[
    "top-left",
    "top",
    "top-right",
    "left",
    "center",
    "right",
    "bottom-left",
    "bottom",
    "bottom-right",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub schema_version: u32,
    pub microphone: MicrophoneSettings,
    pub hotkeys: Vec<Hotkey>,
    pub general: GeneralSettings,
    /// Missing on files saved before the toast existed. Schema stays 1.
    #[serde(default)]
    pub toast: ToastSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MicrophoneSettings {
    pub target: String,
    pub mute_on_start: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hotkey {
    pub id: String,
    pub shortcut: String,
    pub action: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneralSettings {
    pub start_minimized: bool,
    pub close_to_tray: bool,
    pub start_with_system: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToastSettings {
    #[serde(default = "default_toast_enabled")]
    pub enabled: bool,
    #[serde(default = "default_toast_position")]
    pub position: String,
    #[serde(default)]
    pub sound: String,
}

fn default_toast_enabled() -> bool {
    true
}

fn default_toast_position() -> String {
    "bottom".to_string()
}

impl Default for ToastSettings {
    fn default() -> Self {
        Self {
            enabled: default_toast_enabled(),
            position: default_toast_position(),
            sound: String::new(),
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SettingsError {
    #[error("Settings JSON is invalid: {0}")]
    Invalid(String),

    #[error("Unsupported settings schema version {0}")]
    UnsupportedVersion(u32),
}

impl Settings {
    /// The enabled `toggle_mute` shortcut string, registered as written.
    pub fn toggle_shortcut(&self) -> Option<&str> {
        self.hotkeys.iter().find_map(|hotkey| {
            (hotkey.enabled && hotkey.action == "toggle_mute").then_some(hotkey.shortcut.as_str())
        })
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            microphone: MicrophoneSettings {
                target: DEFAULT_DEVICE_ID.to_string(),
                mute_on_start: false,
            },
            hotkeys: vec![Hotkey {
                id: "toggle-default-mic".to_string(),
                shortcut: "F9".to_string(),
                action: "toggle_mute".to_string(),
                enabled: true,
            }],
            general: GeneralSettings {
                start_minimized: true,
                close_to_tray: true,
                start_with_system: false,
            },
            toast: ToastSettings::default(),
        }
    }
}

/// Parse then prepare. Load and save both use this. A failure must not rewrite the file.
pub fn accept_settings(text: &str) -> Result<Settings, SettingsError> {
    prepare_settings(parse_settings(text)?)
}

pub fn parse_settings(text: &str) -> Result<Settings, SettingsError> {
    let settings: Settings =
        serde_json::from_str(text).map_err(|error| SettingsError::Invalid(error.to_string()))?;
    if settings.schema_version != SCHEMA_VERSION {
        return Err(SettingsError::UnsupportedVersion(settings.schema_version));
    }
    Ok(settings)
}

/// Schema stays at 1. A saved v0.1 file is already that shape, so loading it is the migration.
/// Empty shortcuts are dropped. The microphone target stays `default`.
pub fn prepare_settings(mut settings: Settings) -> Result<Settings, SettingsError> {
    settings.microphone.target = DEFAULT_DEVICE_ID.to_string();
    settings
        .hotkeys
        .retain(|hotkey| !hotkey.shortcut.trim().is_empty());
    let mut seen = Vec::new();
    for (index, hotkey) in settings.hotkeys.iter_mut().enumerate() {
        hotkey.shortcut = hotkey.shortcut.trim().to_string();
        if hotkey.id.trim().is_empty() {
            hotkey.id = format!("hotkey-{index}");
        }
        if !ACTIONS.contains(&hotkey.action.as_str()) {
            return Err(SettingsError::Invalid(format!(
                "Unknown hotkey action {}",
                hotkey.action
            )));
        }
        let parsed = HotKey::from_str(&hotkey.shortcut).map_err(|_| {
            SettingsError::Invalid(format!(
                "Shortcut {} could not be registered",
                hotkey.shortcut
            ))
        })?;
        if !hotkey.enabled {
            continue;
        }
        let key = parsed.id();
        if seen.contains(&key) {
            return Err(SettingsError::Invalid(format!(
                "Shortcut {} is used more than once",
                hotkey.shortcut
            )));
        }
        seen.push(key);
    }
    settings.toast.position = settings.toast.position.trim().to_string();
    settings.toast.sound = settings.toast.sound.trim().to_string();
    if !TOAST_POSITIONS.contains(&settings.toast.position.as_str()) {
        return Err(SettingsError::Invalid(format!(
            "Unknown toast position {}",
            settings.toast.position
        )));
    }
    if settings.toast.sound.contains(['\n', '\r']) {
        return Err(SettingsError::Invalid(
            "The sound path is not valid".to_string(),
        ));
    }
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broken_json_returns_an_error() {
        for text in ["", "{", "null", "not-json", "{\"schema_version\":"] {
            assert!(parse_settings(text).is_err(), "{text}");
        }
    }

    #[test]
    fn toggle_shortcut_uses_the_enabled_toggle() {
        assert_eq!(Settings::default().toggle_shortcut(), Some("F9"));

        let mut settings = Settings::default();
        settings.hotkeys[0].enabled = false;
        assert_eq!(settings.toggle_shortcut(), None);
    }

    #[test]
    fn default_json_matches_schema_v1() {
        let value = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["microphone"]["target"], "default");
        assert_eq!(value["microphone"]["mute_on_start"], false);
        assert_eq!(value["hotkeys"].as_array().unwrap().len(), 1);
        assert_eq!(value["hotkeys"][0]["shortcut"], "F9");
        assert_eq!(value["hotkeys"][0]["action"], "toggle_mute");
        assert_eq!(value["general"]["start_minimized"], true);
        assert_eq!(value["general"]["close_to_tray"], true);
        assert_eq!(value["general"]["start_with_system"], false);
    }

    #[test]
    fn saved_v1_without_toast_still_loads() {
        let text = r#"{"schema_version":1,"microphone":{"target":"default","mute_on_start":false},"hotkeys":[{"id":"toggle-default-mic","shortcut":"F9","action":"toggle_mute","enabled":true}],"general":{"start_minimized":true,"close_to_tray":true,"start_with_system":false}}"#;
        let settings = parse_settings(text).unwrap();
        assert!(settings.toast.enabled);
        assert_eq!(settings.toast.position, "bottom");
        assert!(settings.toast.sound.is_empty());
    }

    #[test]
    fn prepare_rejects_an_unknown_toast_position() {
        let mut settings = Settings::default();
        settings.toast.position = "middle".to_string();
        assert!(prepare_settings(settings).is_err());
    }

    #[test]
    fn default_settings_round_trip() {
        let text = serde_json::to_string(&Settings::default()).unwrap();
        assert_eq!(parse_settings(&text).unwrap(), Settings::default());
    }

    #[test]
    fn prepare_keeps_several_shortcuts_for_one_action() {
        let mut settings = Settings::default();
        settings.microphone.target = "custom".to_string();
        settings.hotkeys.push(Hotkey {
            id: String::new(),
            shortcut: " Ctrl+Shift+F9 ".to_string(),
            action: "toggle_mute".to_string(),
            enabled: true,
        });
        settings.hotkeys.push(Hotkey {
            id: "blank".to_string(),
            shortcut: " ".to_string(),
            action: "force_mute".to_string(),
            enabled: true,
        });
        let settings = prepare_settings(settings).unwrap();
        assert_eq!(settings.microphone.target, "default");
        assert_eq!(settings.hotkeys.len(), 2);
        assert_eq!(settings.hotkeys[1].shortcut, "Ctrl+Shift+F9");
        assert_eq!(settings.hotkeys[1].id, "hotkey-1");
    }

    #[test]
    fn accept_settings_prepares_on_load() {
        let old = r#"{"schema_version":1,"microphone":{"target":"custom","mute_on_start":false},"hotkeys":[{"id":"toggle-default-mic","shortcut":"F9","action":"toggle_mute","enabled":true}],"general":{"start_minimized":true,"close_to_tray":true,"start_with_system":false}}"#;
        let settings = accept_settings(old).unwrap();
        assert_eq!(settings.microphone.target, "default");
        assert!(settings.toast.enabled);

        let duplicate = r#"{"schema_version":1,"microphone":{"target":"default","mute_on_start":false},"hotkeys":[{"id":"a","shortcut":"Ctrl+Shift+F9","action":"toggle_mute","enabled":true},{"id":"b","shortcut":"Shift+Ctrl+F9","action":"force_mute","enabled":true}],"general":{"start_minimized":true,"close_to_tray":true,"start_with_system":false},"toast":{"enabled":true,"position":"bottom","sound":""}}"#;
        assert!(accept_settings(duplicate).is_err());

        let broken_key = r#"{"schema_version":1,"microphone":{"target":"default","mute_on_start":false},"hotkeys":[{"id":"a","shortcut":"Nope","action":"toggle_mute","enabled":true}],"general":{"start_minimized":true,"close_to_tray":true,"start_with_system":false}}"#;
        assert!(accept_settings(broken_key).is_err());
        assert!(accept_settings("{").is_err());
    }

    #[test]
    fn prepare_rejects_a_duplicate_or_unknown_action() {
        let mut settings = Settings::default();
        settings.hotkeys.push(Hotkey {
            id: "again".to_string(),
            shortcut: "f9".to_string(),
            action: "force_mute".to_string(),
            enabled: true,
        });
        assert!(prepare_settings(settings).is_err());

        let mut settings = Settings::default();
        settings.hotkeys[0].action = "force-mute".to_string();
        assert!(prepare_settings(settings).is_err());
    }
}
