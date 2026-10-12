use std::sync::Mutex;

use mic_core::{DEFAULT_DEVICE_ID, toggle_mute};
use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutEvent, ShortcutState};

struct HotkeyNotice {
    shortcut: String,
    error: Mutex<String>,
}

#[derive(serde::Serialize)]
struct ShortcutStatus {
    shortcut: String,
    error: String,
}

fn notice_error(notice: &HotkeyNotice) -> String {
    notice
        .error
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

fn store_notice(app: &tauri::AppHandle, message: String) {
    let notice = app.state::<HotkeyNotice>();
    *notice
        .error
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = message.clone();
    let _ = app.emit("hotkey-error", message);
}

fn on_hotkey(
    app: &tauri::AppHandle,
    _shortcut: &tauri_plugin_global_shortcut::Shortcut,
    event: ShortcutEvent,
) {
    if event.state != ShortcutState::Pressed {
        return;
    }
    let controller = mic_platform::open_controller();
    let message = match toggle_mute(controller.as_ref(), DEFAULT_DEVICE_ID) {
        Ok(_) => String::new(),
        Err(error) => format!("Could not toggle the microphone: {error}"),
    };
    store_notice(app, message);
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
fn shortcut_status(notice: tauri::State<'_, HotkeyNotice>) -> ShortcutStatus {
    ShortcutStatus {
        shortcut: notice.shortcut.clone(),
        error: notice_error(&notice),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(on_hotkey)
                .build(),
        )
        .setup(|app| {
            let shortcut = mic_settings::Settings::default()
                .toggle_shortcut()
                .unwrap_or("")
                .to_string();
            app.manage(HotkeyNotice {
                shortcut: shortcut.clone(),
                error: Mutex::new(String::new()),
            });
            if shortcut.is_empty() {
                store_notice(
                    app.handle(),
                    "No toggle shortcut is configured.".to_string(),
                );
            } else if let Err(error) = app.global_shortcut().register(shortcut.as_str()) {
                store_notice(
                    app.handle(),
                    format!("Shortcut {shortcut} could not be registered: {error}"),
                );
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![greet, shortcut_status])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    #[test]
    fn default_shortcut_parses() {
        let settings = mic_settings::Settings::default();
        let text = settings.toggle_shortcut().expect("default toggle shortcut");
        text.parse::<tauri_plugin_global_shortcut::Shortcut>()
            .expect("default shortcut must parse");
    }
}
