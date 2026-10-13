use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use mic_core::{DEFAULT_DEVICE_ID, MuteSession, StatusLabel, mute_status, toggle_mute};
use mic_settings::{Settings, WorkArea, accept_settings, toast_rect, work_area_at};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, WindowEvent, Wry};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};
use tauri_plugin_store::StoreExt;

// Close hides the window. Exit sets this first so that close does not keep the process alive.
static EXITING: AtomicBool = AtomicBool::new(false);
static TOAST_TICK: AtomicU64 = AtomicU64::new(0);

const MAIN_WIDTH: f64 = 1120.0;
const MAIN_HEIGHT: f64 = 760.0;

struct TrayStatus {
    icon: TrayIcon<Wry>,
    label: MenuItem<Wry>,
    toggle: MenuItem<Wry>,
}

const STORE_PATH: &str = "settings.json";

struct HotkeyNotice {
    error: Mutex<String>,
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
    if !message.is_empty() {
        tracing::warn!("{message}");
    }
    let notice = app.state::<HotkeyNotice>();
    *notice
        .error
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = message.clone();
    let _ = app.emit("hotkey-error", message);
}

fn append_notice(app: &tauri::AppHandle, message: String) {
    let prior = app
        .try_state::<HotkeyNotice>()
        .map(|notice| notice_error(&notice))
        .unwrap_or_default();
    let message = if prior.is_empty() {
        message
    } else {
        format!("{prior} {message}")
    };
    store_notice(app, message);
}

fn current_settings(app: &tauri::AppHandle) -> Settings {
    app.state::<Mutex<Settings>>()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

fn with_session<T>(app: &tauri::AppHandle, update: impl FnOnce(&mut MuteSession) -> T) -> T {
    let state = app.state::<Mutex<MuteSession>>();
    let mut session = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    update(&mut session)
}

fn sync_status(app: &tauri::AppHandle) -> String {
    let controller = mic_platform::open_controller();
    let label = StatusLabel::from_mute(mute_status(controller.as_ref(), DEFAULT_DEVICE_ID));
    let text = label.as_str();
    if let Some(tray) = app.try_state::<TrayStatus>() {
        let _ = tray.label.set_text(text);
        let _ = tray.icon.set_tooltip(Some(format!("MICRIX — {text}")));
    }
    let _ = app.emit("mic-status", text);
    let toast = with_session(app, |session| session.observe(label));
    if let Some(toast) = toast {
        present_toast(app, toast);
    }
    text.to_string()
}

fn present_toast(app: &tauri::AppHandle, text: &str) {
    let settings = current_settings(app);
    play_toast_sound(&settings.toast.sound);
    if !settings.toast.enabled {
        return;
    }
    let Some(toast) = app.get_webview_window("toast") else {
        return;
    };
    place_toast(app, &toast, &settings.toast.position);
    let script = format!(
        "window.__micrixToast && window.__micrixToast({})",
        serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string())
    );
    let _ = toast.eval(script);
    let _ = toast.show();
    let tick = TOAST_TICK.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1600));
        if TOAST_TICK.load(Ordering::SeqCst) != tick {
            return;
        }
        if let Some(toast) = app.get_webview_window("toast") {
            let _ = toast.hide();
        }
    });
}

fn place_toast(app: &tauri::AppHandle, toast: &tauri::WebviewWindow, position: &str) {
    let Some(monitor) = toast_monitor(app) else {
        return;
    };
    let area = monitor.work_area();
    let rect = toast_rect(
        WorkArea {
            x: area.position.x,
            y: area.position.y,
            width: area.size.width,
            height: area.size.height,
        },
        monitor.scale_factor(),
        position,
    );
    let _ = toast.set_size(tauri::Size::Physical(tauri::PhysicalSize::new(
        rect.width,
        rect.height,
    )));
    let _ = toast.set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(
        rect.x, rect.y,
    )));
}

fn toast_monitor(app: &tauri::AppHandle) -> Option<tauri::Monitor> {
    let cursor = app
        .cursor_position()
        .ok()
        .map(|cursor| (cursor.x as i32, cursor.y as i32));
    let monitors = app.available_monitors().unwrap_or_default();
    let areas: Vec<WorkArea> = monitors
        .iter()
        .map(|monitor| {
            let area = monitor.work_area();
            WorkArea {
                x: area.position.x,
                y: area.position.y,
                width: area.size.width,
                height: area.size.height,
            }
        })
        .collect();
    if let Some(picked) = work_area_at(cursor, &areas) {
        return monitors.into_iter().find(|monitor| {
            let area = monitor.work_area();
            area.position.x == picked.x
                && area.position.y == picked.y
                && area.size.width == picked.width
                && area.size.height == picked.height
        });
    }
    app.primary_monitor().ok().flatten()
}

fn play_toast_sound(path: &str) {
    if path.is_empty() || !std::path::Path::new(path).is_file() {
        return;
    }
    #[cfg(windows)]
    {
        use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_FILENAME, SND_NODEFAULT};
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            let _ = PlaySoundW(
                windows::core::PCWSTR(wide.as_ptr()),
                None,
                SND_FILENAME | SND_ASYNC | SND_NODEFAULT,
            );
        }
    }
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
            accept_settings(&text).map_err(|error| error.to_string())
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
    let Some(restore) = with_session(app, |session| session.release()) else {
        return;
    };
    if !restore {
        return;
    }
    let controller = mic_platform::open_controller();
    if let Err(error) = controller.set_muted(DEFAULT_DEVICE_ID, false) {
        store_notice(app, format!("Could not restore the microphone: {error}"));
    }
    sync_status(app);
}

fn begin_hold(app: &tauri::AppHandle, shortcut: &Shortcut) {
    enum HoldStart {
        Busy,
        Failed(String),
        MuteFailed(String),
        Done,
    }
    let started = with_session(app, |slot| {
        if !slot.can_begin_hold() {
            return HoldStart::Busy;
        }
        let controller = mic_platform::open_controller();
        let muted = match controller.get_muted(DEFAULT_DEVICE_ID) {
            Ok(muted) => muted,
            Err(error) => {
                return HoldStart::Failed(format!("Could not mute while held: {error}"));
            }
        };
        if !muted && let Err(error) = controller.set_muted(DEFAULT_DEVICE_ID, true) {
            return HoldStart::MuteFailed(format!("Could not mute while held: {error}"));
        }
        let saved = current_settings(app)
            .hotkeys
            .into_iter()
            .find_map(|hotkey| {
                (hotkey.enabled && same_shortcut(&hotkey.shortcut, shortcut))
                    .then_some(hotkey.shortcut)
            })
            .unwrap_or_default();
        slot.commit_hold(saved, muted);
        HoldStart::Done
    });
    match started {
        HoldStart::Busy => {}
        HoldStart::Failed(message) => store_notice(app, message),
        HoldStart::MuteFailed(message) => {
            store_notice(app, message);
            sync_status(app);
        }
        HoldStart::Done => {
            sync_status(app);
        }
    }
}

fn end_hold(app: &tauri::AppHandle, shortcut: &Shortcut) {
    let matches = with_session(app, |session| {
        session
            .hold()
            .is_some_and(|held| same_shortcut(&held.shortcut, shortcut))
    });
    if !matches {
        return;
    }
    release_hold(app);
}

#[cfg(windows)]
fn allow_shortcut_recording(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    // WebView2 swallows browser accelerators (Ctrl+J opens Downloads) before the page can record them.
    let _ = window.with_webview(|webview| unsafe {
        use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings3;
        use windows_core::Interface;
        let Ok(core) = webview.controller().CoreWebView2() else {
            return;
        };
        let Ok(settings) = core.Settings() else {
            return;
        };
        let Ok(settings) = settings.cast::<ICoreWebView2Settings3>() else {
            return;
        };
        let _ = settings.SetAreBrowserAcceleratorKeysEnabled(false);
    });
}

#[cfg(not(windows))]
fn allow_shortcut_recording(_app: &tauri::AppHandle) {}

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
        append_notice(app, format!("Could not update startup: {error}"));
    }
}

fn mute_if_requested(app: &tauri::AppHandle, settings: &Settings) {
    if !settings.microphone.mute_on_start {
        return;
    }
    let controller = mic_platform::open_controller();
    if let Err(error) = controller.set_muted(DEFAULT_DEVICE_ID, true) {
        append_notice(app, format!("Could not mute on start: {error}"));
    }
}

fn stop_watch(app: &tauri::AppHandle) {
    let Some(slot) = app.try_state::<Mutex<Option<mic_platform::EndpointWatch>>>() else {
        return;
    };
    slot.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
}

fn start_watch(app: &tauri::AppHandle) {
    let handle = app.clone();
    match mic_platform::watch_default_input(move || {
        let handle = handle.clone();
        // OnNotify can run on the main thread. A direct hop would call GetMute inside that callback.
        std::thread::spawn(move || {
            let refresh = handle.clone();
            if let Err(error) = handle.run_on_main_thread(move || {
                sync_status(&refresh);
            }) {
                tracing::warn!("could not refresh mute status: {error}");
            }
        });
    }) {
        Ok(watch) => {
            app.manage(Mutex::new(Some(watch)));
        }
        Err(error) => append_notice(app, format!("Could not watch the microphone: {error}")),
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
    stop_watch(app);
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
    let settings = accept_settings(&settings_json).map_err(|error| error.to_string())?;
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

#[tauri::command]
fn pick_sound(app: tauri::AppHandle) -> Result<String, String> {
    let Some(window) = app.get_webview_window("main") else {
        return Err("The window is not open.".to_string());
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    let dialog_window = window.clone();
    window
        .run_on_main_thread(move || {
            let _ = sender.send(choose_wav(window_owner(&dialog_window)));
        })
        .map_err(|error| error.to_string())?;
    receiver.recv().map_err(|error| error.to_string())?
}

fn window_owner(window: &tauri::WebviewWindow) -> Option<isize> {
    #[cfg(windows)]
    {
        window.hwnd().ok().map(|handle| handle.0 as isize)
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        None
    }
}

fn choose_wav(owner: Option<isize>) -> Result<String, String> {
    #[cfg(windows)]
    {
        use std::mem::size_of;
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::Controls::Dialogs::{
            GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_PATHMUSTEXIST, OPENFILENAMEW,
        };
        use windows::core::{PCWSTR, PWSTR};

        let mut buffer = vec![0u16; 1024];
        let filter: Vec<u16> = "Sound (WAV)\0*.wav\0\0".encode_utf16().collect();
        let title: Vec<u16> = "Add a sound\0".encode_utf16().collect();
        let extension: Vec<u16> = "wav\0".encode_utf16().collect();
        let mut dialog: OPENFILENAMEW = unsafe { std::mem::zeroed() };
        dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
        dialog.hwndOwner = HWND(owner.unwrap_or(0) as *mut _);
        dialog.lpstrFilter = PCWSTR(filter.as_ptr());
        dialog.lpstrFile = PWSTR(buffer.as_mut_ptr());
        dialog.nMaxFile = buffer.len() as u32;
        dialog.lpstrTitle = PCWSTR(title.as_ptr());
        dialog.lpstrDefExt = PCWSTR(extension.as_ptr());
        dialog.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST;
        let chosen = unsafe { GetOpenFileNameW(&mut dialog) };
        if !chosen.as_bool() {
            return Ok(String::new());
        }
        let end = buffer
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(buffer.len());
        Ok(String::from_utf16_lossy(&buffer[..end]))
    }
    #[cfg(not(windows))]
    {
        let _ = owner;
        Err("Sound files can be chosen on Windows.".to_string())
    }
}

struct LogSink {
    file: Option<std::fs::File>,
}

impl Write for LogSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = std::io::stderr().write_all(buf);
        if let Some(file) = &mut self.file {
            let _ = file.write_all(buf);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let _ = std::io::stderr().flush();
        if let Some(file) = &mut self.file {
            let _ = file.flush();
        }
        Ok(())
    }
}

fn log_file() -> Option<std::fs::File> {
    // shortcut: micrix.log only appends, rotate it if it grows past a few megabytes
    let dir = PathBuf::from(std::env::var_os("APPDATA")?).join("com.micrix.app");
    std::fs::create_dir_all(&dir).ok()?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("micrix.log"))
        .ok()
}

fn init_tracing() {
    let file = Mutex::new(log_file());
    let _ = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .with_writer(move || LogSink {
            file: file
                .lock()
                .ok()
                .and_then(|slot| slot.as_ref().and_then(|handle| handle.try_clone().ok())),
        })
        .try_init();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();
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
            app.manage(Mutex::new(MuteSession::new()));
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
            start_watch(app.handle());
            mute_if_requested(app.handle(), &settings);
            sync_status(app.handle());
            allow_shortcut_recording(app.handle());
            if let Some(toast) = app.get_webview_window("toast") {
                let _ = toast.set_ignore_cursor_events(true);
            }
            // Config starts the window hidden. Showing here is the only path that respects start_minimized.
            if !settings.general.start_minimized
                && let Some(window) = app.get_webview_window("main")
            {
                let _ = window.show();
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
                return;
            }
            match event {
                WindowEvent::CloseRequested { api, .. } => {
                    if EXITING.load(Ordering::SeqCst) || !hides_on_close(window) {
                        release_hold(window.app_handle());
                        stop_watch(window.app_handle());
                        return;
                    }
                    api.prevent_close();
                    let _ = window.hide();
                }
                WindowEvent::Resized(size) => {
                    // Hiding the window reports 0. A maximize or snap still changes the client size.
                    if size.width == 0 || size.height == 0 {
                        return;
                    }
                    let Ok(scale) = window.scale_factor() else {
                        return;
                    };
                    let width = f64::from(size.width) / scale;
                    let height = f64::from(size.height) / scale;
                    if corrected_main_size(width, height).is_none() {
                        return;
                    }
                    let _ = window.unmaximize();
                    let _ = window.set_fullscreen(false);
                    let _ = window.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
                        MAIN_WIDTH,
                        MAIN_HEIGHT,
                    )));
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            snapshot,
            save_settings,
            toggle_microphone,
            pick_sound
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// `None` keeps the reported logical size. A hit asks the shell to snap back.
fn corrected_main_size(width: f64, height: f64) -> Option<(f64, f64)> {
    if width == 0.0 || height == 0.0 {
        return None;
    }
    if (width - MAIN_WIDTH).abs() < 2.0 && (height - MAIN_HEIGHT).abs() < 2.0 {
        return None;
    }
    Some((MAIN_WIDTH, MAIN_HEIGHT))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_window_stays_at_the_fixed_size() {
        assert_eq!(corrected_main_size(0.0, 760.0), None);
        assert_eq!(corrected_main_size(1120.0, 760.0), None);
        assert_eq!(corrected_main_size(1121.5, 760.0), None);
        assert_eq!(
            corrected_main_size(800.0, 600.0),
            Some((MAIN_WIDTH, MAIN_HEIGHT))
        );
    }

    #[test]
    fn default_shortcut_parses() {
        let settings = mic_settings::Settings::default();
        let text = settings.toggle_shortcut().expect("default toggle shortcut");
        text.parse::<tauri_plugin_global_shortcut::Shortcut>()
            .expect("default shortcut must parse");
    }
}
