use mic_core::DEFAULT_DEVICE_ID;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const SCHEMA_VERSION: u32 = 1;

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
    fn default_settings_round_trip() {
        let text = serde_json::to_string(&Settings::default()).unwrap();
        assert_eq!(parse_settings(&text).unwrap(), Settings::default());
    }
}
