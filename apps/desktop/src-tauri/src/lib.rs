use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use mic_core::{DEFAULT_DEVICE_ID, MicError, MuteStatus, mute_status, toggle_mute};
use mic_settings::{Settings, parse_settings, prepare_settings};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, WindowEvent, Wry};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};
use tauri_plugin_store::StoreExt;

// Close hides the window. Exit sets this first so that close does not keep the process alive.
static EXITING: AtomicBool = AtomicBool::new(false);

struct TrayStatus {
    icon: TrayIcon<Wry>,
    label: MenuItem<Wry>,
    toggle: MenuItem<Wry>,
}

const STORE_PATH: &str = "settings.json";

struct HotkeyNotice {
    error: Mutex<String>,
}

struct HeldMute {
    shortcut: String,
    restore_to_unmuted: bool,
}

#[derive(serde::Serialize)]
struct Snapshot {
    status: String,
    error: String,
    settings_json: String,
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

fn current_settings(app: &tauri::AppHandle) -> Settings {
    app.state::<Mutex<Settings>>()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

fn sync_status(app: &tauri::AppHandle) -> String {
    let controller = mic_platform::open_controller();
    let label = status_label(mute_status(controller.as_ref(), DEFAULT_DEVICE_ID)).to_string();
    if let Some(tray) = app.try_state::<TrayStatus>() {
        let _ = tray.label.set_text(&label);
        let _ = tray.icon.set_tooltip(Some(format!("MICRIX — {label}")));
    }
    let _ = app.emit("mic-status", &label);
    label
}

fn toggle_menu_text(shortcut: &str) -> String {
    if shortcut.is_empty() {
        "Toggle Microphone".to_string()
    } else {
        format!("Toggle Microphone\t{shortcut}")
    }
}

fn hides_on_close(window: &tauri::Window) -> bool {
    window
        .try_state::<Mutex<Settings>>()
        .and_then(|settings| {
            settings
                .lock()
                .ok()
                .map(|settings| settings.general.close_to_tray)
        })
        .unwrap_or(true)
}

fn load_or_create_settings(app: &tauri::App) -> Result<Settings, String> {
    let store = app.store(STORE_PATH).map_err(|error| error.to_string())?;
    match store.get("settings") {
        None => {
            let settings = Settings::default();
            let value = serde_json::to_value(&settings).map_err(|error| error.to_string())?;
            store.set("settings", value);
            store.save().map_err(|error| error.to_string())?;
            Ok(settings)
        }
        Some(value) => {
            let text = serde_json::to_string(&value).map_err(|error| error.to_string())?;
            parse_settings(&text).map_err(|error| error.to_string())
        }
    }
}

fn write_settings(app: &tauri::AppHandle, settings: &Settings) -> Result<(), String> {
    let store = app.store(STORE_PATH).map_err(|error| error.to_string())?;
    let value = serde_json::to_value(settings).map_err(|error| error.to_string())?;
    store.set("settings", value);
    store.save().map_err(|error| error.to_string())?;
    Ok(())
}

fn same_shortcut(saved: &str, event: &Shortcut) -> bool {
    saved.parse::<Shortcut>().ok().as_ref() == Some(event)
}

fn release_hold(app: &tauri::AppHandle) {
    let Some(held) = app
        .try_state::<Mutex<Option<HeldMute>>>()
        .and_then(|slot| slot.lock().ok().and_then(|mut slot| slot.take()))
    else {
        return;
    };
    if !held.restore_to_unmuted {
        return;
    }
    let controller = mic_platform::open_controller();
    if let Err(error) = controller.set_muted(DEFAULT_DEVICE_ID, false) {
        store_notice(app, format!("Could not restore the microphone: {error}"));
    }
    sync_status(app);
}

fn begin_hold(app: &tauri::AppHandle, shortcut: &Shortcut) {
    let Some(slot) = app.try_state::<Mutex<Option<HeldMute>>>() else {
        return;
    };
    let mut slot = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if slot.is_some() {
        return;
    }
    let controller = mic_platform::open_controller();
    let muted = match controller.get_muted(DEFAULT_DEVICE_ID) {
        Ok(muted) => muted,
        Err(error) => {
            drop(slot);
            store_notice(app, format!("Could not mute while held: {error}"));
            return;
        }
    };
    if !muted && let Err(error) = controller.set_muted(DEFAULT_DEVICE_ID, true) {
        drop(slot);
        store_notice(app, format!("Could not mute while held: {error}"));
        sync_status(app);
        return;
    }
    let saved = current_settings(app)
        .hotkeys
        .into_iter()
        .find_map(|hotkey| {
            (hotkey.enabled && same_shortcut(&hotkey.shortcut, shortcut)).then_some(hotkey.shortcut)
        })
        .unwrap_or_default();
    *slot = Some(HeldMute {
        shortcut: saved,
        restore_to_unmuted: !muted,
    });
    drop(slot);
    sync_status(app);
}

fn end_hold(app: &tauri::AppHandle, shortcut: &Shortcut) {
    let Some(slot) = app.try_state::<Mutex<Option<HeldMute>>>() else {
        return;
    };
    let slot = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(held) = slot.as_ref() else {
        return;
    };
    if !same_shortcut(&held.shortcut, shortcut) {
        return;
    }
    drop(slot);
    release_hold(app);
}

fn apply_hotkey(app: &tauri::AppHandle, settings: &Settings) {
    release_hold(app);
    let _ = app.global_shortcut().unregister_all();
    let mut errors = Vec::new();
    let mut registered = 0;
    for hotkey in settings
        .hotkeys
        .iter()
        .filter(|hotkey| hotkey.enabled && !hotkey.shortcut.is_empty())
    {
        match app.global_shortcut().register(hotkey.shortcut.as_str()) {
            Ok(()) => registered += 1,
            Err(error) => {
                errors.push(format!(
                    "Shortcut {} could not be registered: {error}",
                    hotkey.shortcut
                ));
            }
        }
    }
    let toggle = settings.toggle_shortcut().unwrap_or("");
    if let Some(tray) = app.try_state::<TrayStatus>() {
        let _ = tray.toggle.set_text(toggle_menu_text(toggle));
    }
    let message = if !errors.is_empty() {
        errors.join(" ")
    } else if registered == 0 {
        "No shortcut is configured.".to_string()
    } else {
        String::new()
    };
    store_notice(app, message);
}

fn apply_autostart(app: &tauri::AppHandle, settings: &Settings) {
    let manager = app.autolaunch();
    let result = if settings.general.start_with_system {
        manager.enable()
    } else if manager.is_enabled().unwrap_or(false) {
        manager.disable()
    } else {
        Ok(())
    };
    if let Err(error) = result {
        let prior = app
            .try_state::<HotkeyNotice>()
            .map(|notice| notice_error(&notice))
            .unwrap_or_default();
        let message = format!("Could not update startup: {error}");
        let message = if prior.is_empty() {
            message
        } else {
            format!("{prior} {message}")
        };
        store_notice(app, message);
    }
}

fn mute_if_requested(app: &tauri::AppHandle, settings: &Settings) {
    if !settings.microphone.mute_on_start {
        return;
    }
    let controller = mic_platform::open_controller();
    if let Err(error) = controller.set_muted(DEFAULT_DEVICE_ID, true) {
        let prior = app
            .try_state::<HotkeyNotice>()
            .map(|notice| notice_error(&notice))
            .unwrap_or_default();
        let message = format!("Could not mute on start: {error}");
        let message = if prior.is_empty() {
            message
        } else {
            format!("{prior} {message}")
        };
        store_notice(app, message);
    }
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
    release_hold(app);
    app.exit(0);
}

fn set_default(app: &tauri::AppHandle, muted: bool) {
    let controller = mic_platform::open_controller();
    let message = match controller.set_muted(DEFAULT_DEVICE_ID, muted) {
        Ok(()) => String::new(),
        Err(error) => format!("Could not change the microphone: {error}"),
    };
    store_notice(app, message);
    sync_status(app);
}

fn toggle_default(app: &tauri::AppHandle) {
    let controller = mic_platform::open_controller();
    let message = match toggle_mute(controller.as_ref(), DEFAULT_DEVICE_ID) {
        Ok(_) => String::new(),
        Err(error) => format!("Could not toggle the microphone: {error}"),
    };
    store_notice(app, message);
    sync_status(app);
}

fn on_hotkey(app: &tauri::AppHandle, shortcut: &Shortcut, event: ShortcutEvent) {
    let action = current_settings(app)
        .hotkeys
        .into_iter()
        .find_map(|hotkey| {
            (hotkey.enabled && same_shortcut(&hotkey.shortcut, shortcut)).then_some(hotkey.action)
        });
    match (action.as_deref(), event.state) {
        (Some("toggle_mute"), ShortcutState::Pressed) => toggle_default(app),
        (Some("force_mute"), ShortcutState::Pressed) => set_default(app, true),
        (Some("force_unmute"), ShortcutState::Pressed) => set_default(app, false),
        (Some("open_settings"), ShortcutState::Pressed) => show_main(app),
        (Some("hold_to_mute"), ShortcutState::Pressed) => begin_hold(app, shortcut),
        (Some("hold_to_mute"), ShortcutState::Released) => end_hold(app, shortcut),
        _ => {}
    }
}

fn install_tray(app: &mut tauri::App, shortcut: &str) -> tauri::Result<()> {
    let toggle_text = toggle_menu_text(shortcut);
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
                sync_status(tray.app_handle());
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
        toggle,
    });
    Ok(())
}

fn snapshot_of(app: &tauri::AppHandle) -> Snapshot {
    let settings = current_settings(app);
    let settings_json = serde_json::to_string(&settings).unwrap_or_else(|_| "{}".to_string());
    let error = app
        .try_state::<HotkeyNotice>()
        .map(|notice| notice_error(&notice))
        .unwrap_or_default();
    Snapshot {
        status: sync_status(app),
        error,
        settings_json,
    }
}

#[tauri::command]
fn snapshot(app: tauri::AppHandle) -> Snapshot {
    snapshot_of(&app)
}

#[tauri::command]
fn save_settings(app: tauri::AppHandle, settings_json: String) -> Result<Snapshot, String> {
    let settings = parse_settings(&settings_json).map_err(|error| error.to_string())?;
    let settings = prepare_settings(settings).map_err(|error| error.to_string())?;
    write_settings(&app, &settings)?;
    *app.state::<Mutex<Settings>>()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = settings.clone();
    apply_hotkey(&app, &settings);
    apply_autostart(&app, &settings);
    Ok(snapshot_of(&app))
}

#[tauri::command]
fn toggle_microphone(app: tauri::AppHandle) {
    toggle_default(&app);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder =
        tauri::Builder::default().plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app);
        }));
    builder
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(on_hotkey)
                .build(),
        )
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .setup(|app| {
            app.manage(HotkeyNotice {
                error: Mutex::new(String::new()),
            });
            app.manage(Mutex::new(None::<HeldMute>));
            let settings = match load_or_create_settings(app) {
                Ok(settings) => settings,
                Err(error) => {
                    store_notice(app.handle(), error);
                    Settings::default()
                }
            };
            let shortcut = settings.toggle_shortcut().unwrap_or("").to_string();
            app.manage(Mutex::new(settings.clone()));
            install_tray(app, &shortcut)?;
            apply_hotkey(app.handle(), &settings);
            apply_autostart(app.handle(), &settings);
            mute_if_requested(app.handle(), &settings);
            sync_status(app.handle());
            // Config starts the window hidden. Showing here is the only path that respects start_minimized.
            if !settings.general.start_minimized
                && let Some(window) = app.get_webview_window("main")
            {
                let _ = window.show();
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if EXITING.load(Ordering::SeqCst) || !hides_on_close(window) {
                    release_hold(window.app_handle());
                    return;
                }
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            snapshot,
            save_settings,
            toggle_microphone
        ])
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
