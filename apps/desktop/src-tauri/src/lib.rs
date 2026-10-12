use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use mic_core::{DEFAULT_DEVICE_ID, MicError, MuteStatus, mute_status, toggle_mute};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, WindowEvent, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutEvent, ShortcutState};

// Close hides the window. Exit sets this first so that close does not keep the process alive.
static EXITING: AtomicBool = AtomicBool::new(false);

struct TrayStatus {
    icon: TrayIcon<Wry>,
    label: MenuItem<Wry>,
}

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

fn status_label(status: Result<MuteStatus, MicError>) -> &'static str {
    match status {
        Ok(MuteStatus::Muted) => "Muted",
        Ok(MuteStatus::Unmuted) => "Unmuted",
        Ok(MuteStatus::Unavailable) | Err(_) => "Unavailable",
    }
}

fn refresh_tray(app: &tauri::AppHandle) {
    let Some(tray) = app.try_state::<TrayStatus>() else {
        return;
    };
    let controller = mic_platform::open_controller();
    let label = status_label(mute_status(controller.as_ref(), DEFAULT_DEVICE_ID));
    let _ = tray.label.set_text(label);
    let _ = tray.icon.set_tooltip(Some(format!("MICRIX — {label}")));
}

fn show_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn request_exit(app: &tauri::AppHandle) {
    EXITING.store(true, Ordering::SeqCst);
    app.exit(0);
}

fn toggle_default(app: &tauri::AppHandle) {
    let controller = mic_platform::open_controller();
    let message = match toggle_mute(controller.as_ref(), DEFAULT_DEVICE_ID) {
        Ok(_) => String::new(),
        Err(error) => format!("Could not toggle the microphone: {error}"),
    };
    store_notice(app, message);
    refresh_tray(app);
}

fn on_hotkey(
    app: &tauri::AppHandle,
    _shortcut: &tauri_plugin_global_shortcut::Shortcut,
    event: ShortcutEvent,
) {
    if event.state != ShortcutState::Pressed {
        return;
    }
    toggle_default(app);
}

fn install_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let shortcut = mic_settings::Settings::default()
        .toggle_shortcut()
        .unwrap_or("")
        .to_string();
    let toggle_text = if shortcut.is_empty() {
        "Toggle Microphone".to_string()
    } else {
        format!("Toggle Microphone\t{shortcut}")
    };
    let brand = MenuItem::with_id(app, "brand", "MICRIX", false, None::<&str>)?;
    let label = MenuItem::with_id(app, "status", "Unavailable", false, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let toggle = MenuItem::with_id(app, "toggle", toggle_text, true, None::<&str>)?;
    let open_settings =
        MenuItem::with_id(app, "open-settings", "Open Settings", true, None::<&str>)?;
    let configure_hotkeys = MenuItem::with_id(
        app,
        "configure-hotkeys",
        "Configure Hotkeys",
        true,
        None::<&str>,
    )?;
    let exit = MenuItem::with_id(app, "exit", "Exit MICRIX", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &brand,
            &label,
            &separator,
            &toggle,
            &open_settings,
            &configure_hotkeys,
            &exit,
        ],
    )?;
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::AssetNotFound("window icon".into()))?;
    let tray_icon = TrayIconBuilder::new()
        .icon(icon)
        .tooltip("MICRIX")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click { .. } | TrayIconEvent::DoubleClick { .. }
            ) {
                refresh_tray(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| match event.id.as_ref() {
            "toggle" => toggle_default(app),
            "open-settings" | "configure-hotkeys" => show_main(app),
            "exit" => request_exit(app),
            _ => {}
        })
        .build(app)?;
    app.manage(TrayStatus {
        icon: tray_icon,
        label,
    });
    refresh_tray(app.handle());
    Ok(())
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
    let builder =
        tauri::Builder::default().plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app);
        }));
    builder
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
            install_tray(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if EXITING.load(Ordering::SeqCst) {
                    return;
                }
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![greet, shortcut_status])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_shortcut_parses() {
        let settings = mic_settings::Settings::default();
        let text = settings.toggle_shortcut().expect("default toggle shortcut");
        text.parse::<tauri_plugin_global_shortcut::Shortcut>()
            .expect("default shortcut must parse");
    }

    #[test]
    fn tray_status_never_calls_a_failure_unmuted() {
        assert_eq!(status_label(Ok(MuteStatus::Muted)), "Muted");
        assert_eq!(status_label(Ok(MuteStatus::Unmuted)), "Unmuted");
        assert_eq!(status_label(Ok(MuteStatus::Unavailable)), "Unavailable");
        assert_eq!(status_label(Err(MicError::DeviceNotFound)), "Unavailable");
        assert_eq!(
            status_label(Err(MicError::Platform("failed".to_string()))),
            "Unavailable"
        );
    }
}
