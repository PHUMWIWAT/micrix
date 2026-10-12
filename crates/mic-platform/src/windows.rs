use std::cell::Cell;
use std::sync::{Arc, Mutex, Weak};

use mic_core::{DEFAULT_DEVICE_ID, MicError, MicrophoneController, MicrophoneDevice};
use windows::{
    Win32::{
        Foundation::{PROPERTYKEY, RPC_E_CHANGED_MODE},
        Media::Audio::{
            AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE, EDataFlow, ERole,
            Endpoints::{
                IAudioEndpointVolume, IAudioEndpointVolumeCallback,
                IAudioEndpointVolumeCallback_Impl,
            },
            IMMDevice, IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl,
            MMDeviceEnumerator, eAll, eCapture, eConsole,
        },
        System::Com::{
            CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
        },
    },
    core::{GUID, PCWSTR, implement},
};

const E_NOTFOUND: i32 = 0x8007_0490u32 as i32;
const AUDCLNT_E_DEVICE_INVALIDATED: i32 = 0x8889_0004u32 as i32;
// SetMute echoes this context. The watcher skips it; the caller refreshes status itself.
const MICRIX_EVENT: GUID = GUID::from_u128(0x6d69_6372_6978_4000_8000_4d49_4352_4958);

pub struct WindowsController;

impl MicrophoneController for WindowsController {
    fn list_devices(&self) -> Result<Vec<MicrophoneDevice>, MicError> {
        match described(DEFAULT_DEVICE_ID) {
            Ok(device) => Ok(vec![device]),
            Err(MicError::DeviceNotFound) => Ok(Vec::new()),
            Err(error) => Err(error),
        }
    }

    fn get_muted(&self, device_id: &str) -> Result<bool, MicError> {
        let volume = volume(device_id)?;
        unsafe { volume.GetMute() }
            .map(|muted| muted.as_bool())
            .map_err(map_audio_error)
    }

    fn set_muted(&self, device_id: &str, muted: bool) -> Result<(), MicError> {
        let volume = volume(device_id)?;
        unsafe {
            volume
                .SetMute(muted, &MICRIX_EVENT)
                .map_err(map_audio_error)?;
            let actual = volume.GetMute().map_err(map_audio_error)?;
            if actual.as_bool() != muted {
                return Err(MicError::Platform(format!(
                    "endpoint stayed {} after SetMute({muted})",
                    if actual.as_bool() { "muted" } else { "unmuted" }
                )));
            }
        }
        Ok(())
    }
}

fn described(device_id: &str) -> Result<MicrophoneDevice, MicError> {
    let (id, volume) = open_volume(device_id)?;
    let is_muted = unsafe { volume.GetMute() }
        .map(|muted| muted.as_bool())
        .map_err(map_audio_error)?;
    Ok(MicrophoneDevice {
        name: "Default input".to_string(),
        is_default: device_id == DEFAULT_DEVICE_ID,
        is_muted,
        id,
    })
}

fn volume(device_id: &str) -> Result<IAudioEndpointVolume, MicError> {
    Ok(open_volume(device_id)?.1)
}

fn open_volume(device_id: &str) -> Result<(String, IAudioEndpointVolume), MicError> {
    ensure_com()?;
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(map_audio_error)?;
        let device = if device_id == DEFAULT_DEVICE_ID {
            enumerator
                .GetDefaultAudioEndpoint(eCapture, eConsole)
                .map_err(map_audio_error)?
        } else {
            let wide: Vec<u16> = device_id.encode_utf16().chain(std::iter::once(0)).collect();
            enumerator
                .GetDevice(PCWSTR(wide.as_ptr()))
                .map_err(map_audio_error)?
        };
        let id = endpoint_id(&device)?;
        let volume = device
            .Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
            .map_err(map_audio_error)?;
        Ok((id, volume))
    }
}

fn endpoint_id(device: &IMMDevice) -> Result<String, MicError> {
    unsafe {
        let id = device.GetId().map_err(map_audio_error)?;
        if id.is_null() {
            return Err(MicError::Platform("endpoint id was null".to_string()));
        }
        let text = id.to_string().map_err(|error| {
            CoTaskMemFree(Some(id.as_ptr() as *const _));
            MicError::Platform(error.to_string())
        })?;
        CoTaskMemFree(Some(id.as_ptr() as *const _));
        Ok(text)
    }
}

#[allow(clippy::missing_const_for_thread_local)] // clippy misses the const initializer inside the macro
fn ensure_com() -> Result<(), MicError> {
    thread_local! {
        static READY: Cell<bool> = const { Cell::new(false) };
    }
    READY.with(|ready| {
        if ready.get() {
            return Ok(());
        }
        let started = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if started.is_ok() || started == RPC_E_CHANGED_MODE {
            ready.set(true);
            Ok(())
        } else {
            Err(map_audio_error(started.into()))
        }
    })
}

fn map_audio_error(error: windows::core::Error) -> MicError {
    match error.code().0 {
        E_NOTFOUND | AUDCLNT_E_DEVICE_INVALIDATED => MicError::DeviceNotFound,
        _ => MicError::Platform(error.to_string()),
    }
}

/// WASAPI enumerator, volume, and notification objects are free-threaded.
/// The windows crate does not mark those interfaces `Send`.
struct FreeThreaded<T>(T);
unsafe impl<T> Send for FreeThreaded<T> {}
unsafe impl<T> Sync for FreeThreaded<T> {}

struct VolumeBinding {
    volume: FreeThreaded<IAudioEndpointVolume>,
    callback: FreeThreaded<IAudioEndpointVolumeCallback>,
}

struct WatchState {
    enumerator: FreeThreaded<IMMDeviceEnumerator>,
    binding: Mutex<Option<VolumeBinding>>,
    on_change: Arc<dyn Fn() + Send + Sync>,
}

impl WatchState {
    fn rebind(&self) {
        if let Err(error) = ensure_com() {
            tracing::warn!("could not start COM for the endpoint watch: {error}");
            return;
        }
        let mut slot = self
            .binding
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(old) = slot.take() {
            unsafe {
                let _ = old.volume.0.UnregisterControlChangeNotify(&old.callback.0);
            }
        }
        let volume = match open_volume(DEFAULT_DEVICE_ID) {
            Ok((_, volume)) => volume,
            Err(MicError::DeviceNotFound) => {
                tracing::debug!("default input is unavailable");
                return;
            }
            Err(error) => {
                tracing::warn!("could not open the default input: {error}");
                return;
            }
        };
        let callback: IAudioEndpointVolumeCallback = VolumeNotify {
            on_change: Arc::clone(&self.on_change),
        }
        .into();
        if let Err(error) = unsafe { volume.RegisterControlChangeNotify(&callback) } {
            tracing::warn!("could not subscribe to the default input: {error}");
            return;
        }
        *slot = Some(VolumeBinding {
            volume: FreeThreaded(volume),
            callback: FreeThreaded(callback),
        });
    }
}

#[implement(IAudioEndpointVolumeCallback)]
struct VolumeNotify {
    on_change: Arc<dyn Fn() + Send + Sync>,
}

impl IAudioEndpointVolumeCallback_Impl for VolumeNotify_Impl {
    fn OnNotify(&self, notify: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows::core::Result<()> {
        if external_volume_notify(notify) {
            report_change(self.on_change.as_ref());
        }
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct DeviceNotify {
    state: Weak<WatchState>,
}

impl DeviceNotify {
    fn refresh(&self) {
        let Some(state) = self.state.upgrade() else {
            return;
        };
        state.rebind();
        report_change(state.on_change.as_ref());
    }
}

impl IMMNotificationClient_Impl for DeviceNotify_Impl {
    fn OnDeviceStateChanged(
        &self,
        _device_id: &PCWSTR,
        _state: DEVICE_STATE,
    ) -> windows::core::Result<()> {
        self.refresh();
        Ok(())
    }

    fn OnDeviceAdded(&self, _device_id: &PCWSTR) -> windows::core::Result<()> {
        self.refresh();
        Ok(())
    }

    fn OnDeviceRemoved(&self, _device_id: &PCWSTR) -> windows::core::Result<()> {
        self.refresh();
        Ok(())
    }

    fn OnDefaultDeviceChanged(
        &self,
        flow: EDataFlow,
        _role: ERole,
        _device_id: &PCWSTR,
    ) -> windows::core::Result<()> {
        if flow == eCapture || flow == eAll {
            self.refresh();
        }
        Ok(())
    }

    fn OnPropertyValueChanged(
        &self,
        _device_id: &PCWSTR,
        _key: &PROPERTYKEY,
    ) -> windows::core::Result<()> {
        Ok(())
    }
}

fn report_change(on_change: &(dyn Fn() + Send + Sync)) {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(on_change)).is_err() {
        tracing::error!("endpoint notification panicked");
    }
}

fn external_volume_notify(data: *const AUDIO_VOLUME_NOTIFICATION_DATA) -> bool {
    if data.is_null() {
        return true;
    }
    unsafe { (*data).guidEventContext != MICRIX_EVENT }
}

/// Keeps the default-input subscriptions alive. Dropping it unregisters them.
pub struct EndpointWatch {
    state: Arc<WatchState>,
    client: FreeThreaded<IMMNotificationClient>,
}

impl Drop for EndpointWatch {
    fn drop(&mut self) {
        if let Some(binding) = self
            .state
            .binding
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            unsafe {
                let _ = binding
                    .volume
                    .0
                    .UnregisterControlChangeNotify(&binding.callback.0);
            }
        }
        unsafe {
            let _ = self
                .state
                .enumerator
                .0
                .UnregisterEndpointNotificationCallback(&self.client.0);
        }
        tracing::info!("stopped the default input watch");
    }
}

pub fn watch_default_input(
    on_change: impl Fn() + Send + Sync + 'static,
) -> Result<EndpointWatch, MicError> {
    ensure_com()?;
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .map_err(map_audio_error)?;
    let state = Arc::new(WatchState {
        enumerator: FreeThreaded(enumerator),
        binding: Mutex::new(None),
        on_change: Arc::new(on_change),
    });
    let client: IMMNotificationClient = DeviceNotify {
        state: Arc::downgrade(&state),
    }
    .into();
    unsafe {
        state
            .enumerator
            .0
            .RegisterEndpointNotificationCallback(&client)
    }
    .map_err(map_audio_error)?;
    state.rebind();
    tracing::info!("watching the default input");
    Ok(EndpointWatch {
        state,
        client: FreeThreaded(client),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mic_core::toggle_mute;
    use windows::core::HRESULT;

    #[test]
    fn missing_endpoint_is_not_an_unmuted_success() {
        let missing = windows::core::Error::from(HRESULT(E_NOTFOUND));
        assert_eq!(map_audio_error(missing), MicError::DeviceNotFound);
        let invalidated = windows::core::Error::from(HRESULT(AUDCLNT_E_DEVICE_INVALIDATED));
        assert_eq!(map_audio_error(invalidated), MicError::DeviceNotFound);
    }

    #[test]
    fn own_set_mute_is_not_an_external_change() {
        let own = AUDIO_VOLUME_NOTIFICATION_DATA {
            guidEventContext: MICRIX_EVENT,
            ..Default::default()
        };
        assert!(!external_volume_notify(&own));
        let external = AUDIO_VOLUME_NOTIFICATION_DATA::default();
        assert!(external_volume_notify(&external));
        assert!(external_volume_notify(std::ptr::null()));
    }

    #[test]
    #[ignore = "mutes the default input, then restores it"]
    fn default_input_mute_round_trip() {
        let mic = WindowsController;
        let before = mic
            .get_muted(DEFAULT_DEVICE_ID)
            .expect("read default input");
        eprintln!("default input muted={before}");
        let devices = mic.list_devices().expect("list");
        assert!(
            devices
                .iter()
                .any(|device| device.is_default && device.is_muted == before),
            "default input missing from {devices:?}"
        );

        struct Restore {
            original: bool,
        }
        impl Drop for Restore {
            fn drop(&mut self) {
                let _ = WindowsController.set_muted(DEFAULT_DEVICE_ID, self.original);
            }
        }

        mic.set_muted(DEFAULT_DEVICE_ID, !before)
            .expect("set opposite mute");
        let _restore = Restore { original: before };
        assert_eq!(
            mic.get_muted(DEFAULT_DEVICE_ID).expect("read back"),
            !before
        );

        let restored = toggle_mute(&mic, DEFAULT_DEVICE_ID).expect("toggle back");
        assert_eq!(restored, before);
        assert_eq!(
            mic.get_muted(DEFAULT_DEVICE_ID).expect("final read"),
            before
        );
        eprintln!("default input restored muted={before}");
    }

    #[test]
    #[ignore = "mutes the default input from outside MICRIX, then restores it"]
    fn external_mute_notifies_the_watcher() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::{Duration, Instant};

        let before = WindowsController
            .get_muted(DEFAULT_DEVICE_ID)
            .expect("read default input");
        struct Restore(bool);
        impl Drop for Restore {
            fn drop(&mut self) {
                if let Ok(volume) = volume(DEFAULT_DEVICE_ID) {
                    unsafe {
                        let _ = volume.SetMute(self.0, std::ptr::null());
                    }
                }
            }
        }
        let _restore = Restore(before);

        let hit = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&hit);
        let watch = watch_default_input(move || flag.store(true, Ordering::SeqCst))
            .expect("watch default input");
        hit.store(false, Ordering::SeqCst);

        let volume = volume(DEFAULT_DEVICE_ID).expect("open default input");
        unsafe { volume.SetMute(!before, std::ptr::null()) }.expect("external mute");
        let started = Instant::now();
        while !hit.load(Ordering::SeqCst) && started.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            hit.load(Ordering::SeqCst),
            "watcher missed an external mute"
        );
        drop(watch);
    }
}
