#[cfg(not(windows))]
mod unsupported;

#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
mod macos;

use mic_core::MicrophoneController;

#[cfg(windows)]
pub use windows::{EndpointWatch, watch_default_input};

#[cfg(not(windows))]
pub struct EndpointWatch;

/// Subscribes to default-input mute and device changes. Non-Windows builds return [`mic_core::MicError::Unsupported`].
#[cfg(not(windows))]
pub fn watch_default_input(
    on_change: impl Fn() + Send + Sync + 'static,
) -> Result<EndpointWatch, mic_core::MicError> {
    let _ = on_change;
    Err(mic_core::MicError::Unsupported)
}

/// Controller for this operating system. Non-Windows builds return [`mic_core::MicError::Unsupported`].
pub fn open_controller() -> Box<dyn MicrophoneController> {
    #[cfg(windows)]
    {
        Box::new(windows::WindowsController)
    }
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::LinuxController)
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacController)
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        Box::new(unsupported::UnsupportedController)
    }
}
