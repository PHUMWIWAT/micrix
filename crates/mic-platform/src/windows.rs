use std::cell::Cell;

use mic_core::{DEFAULT_DEVICE_ID, MicError, MicrophoneController, MicrophoneDevice};
use windows::{
    Win32::{
        Foundation::RPC_E_CHANGED_MODE,
        Media::Audio::{
            Endpoints::IAudioEndpointVolume, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
            eCapture, eConsole,
        },
        System::Com::{
            CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
        },
    },
    core::PCWSTR,
};

const E_NOTFOUND: i32 = 0x8007_0490u32 as i32;
const AUDCLNT_E_DEVICE_INVALIDATED: i32 = 0x8889_0004u32 as i32;

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
                .SetMute(muted, std::ptr::null())
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

#[cfg(test)]
mod tests {
    use super::*;
    use mic_core::toggle_mute;
    use windows::core::HRESULT;

    #[test]
    fn missing_endpoint_is_not_an_unmuted_success() {
        let missing = windows::core::Error::from(HRESULT(E_NOTFOUND));
        assert_eq!(map_audio_error(missing), MicError::DeviceNotFound);
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
}
