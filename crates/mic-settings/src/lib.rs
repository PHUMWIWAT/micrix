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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub schema_version: u32,
    pub microphone: MicrophoneSettings,
    pub hotkeys: Vec<Hotkey>,
    pub general: GeneralSettings,
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
        }
    }
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
        if !hotkey.enabled {
            continue;
        }
        let key = hotkey.shortcut.to_ascii_lowercase();
        if seen.contains(&key) {
            return Err(SettingsError::Invalid(format!(
                "Shortcut {} is used more than once",
                hotkey.shortcut
            )));
        }
        seen.push(key);
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
