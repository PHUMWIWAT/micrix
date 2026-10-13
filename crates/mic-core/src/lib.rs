mod session;

pub use session::{Hold, MuteSession, StatusLabel};

use thiserror::Error;

/// Settings target and controller id for the current OS default input.
pub const DEFAULT_DEVICE_ID: &str = "default";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicrophoneDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub is_muted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MuteStatus {
    Muted,
    Unmuted,
    Unavailable,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MicError {
    #[error("Microphone device was not found")]
    DeviceNotFound,

    #[error("This platform does not support the requested operation")]
    Unsupported,

    #[error("Audio operation failed: {0}")]
    Platform(String),
}

pub trait MicrophoneController: Send + Sync {
    fn list_devices(&self) -> Result<Vec<MicrophoneDevice>, MicError>;

    fn get_muted(&self, device_id: &str) -> Result<bool, MicError>;

    fn set_muted(&self, device_id: &str, muted: bool) -> Result<(), MicError>;
}

pub fn mute_status(
    controller: &dyn MicrophoneController,
    device_id: &str,
) -> Result<MuteStatus, MicError> {
    match controller.get_muted(device_id) {
        Ok(true) => Ok(MuteStatus::Muted),
        Ok(false) => Ok(MuteStatus::Unmuted),
        Err(MicError::DeviceNotFound) => Ok(MuteStatus::Unavailable),
        Err(error) => Err(error),
    }
}

/// Reads the latest mute bit, then writes the opposite. The returned bool is the state just written.
pub fn toggle_mute(
    controller: &dyn MicrophoneController,
    device_id: &str,
) -> Result<bool, MicError> {
    let next = !controller.get_muted(device_id)?;
    controller.set_muted(device_id, next)?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct FakeMic {
        devices: Mutex<HashMap<String, bool>>,
    }

    impl FakeMic {
        fn new(devices: &[(&str, bool)]) -> Self {
            Self {
                devices: Mutex::new(
                    devices
                        .iter()
                        .map(|(id, muted)| ((*id).to_string(), *muted))
                        .collect(),
                ),
            }
        }

        fn remove(&self, device_id: &str) {
            self.devices.lock().expect("fake mic").remove(device_id);
        }
    }

    impl MicrophoneController for FakeMic {
        fn list_devices(&self) -> Result<Vec<MicrophoneDevice>, MicError> {
            let devices = self.devices.lock().expect("fake mic");
            Ok(devices
                .iter()
                .map(|(id, muted)| MicrophoneDevice {
                    id: id.clone(),
                    name: id.clone(),
                    is_default: id == "default",
                    is_muted: *muted,
                })
                .collect())
        }

        fn get_muted(&self, device_id: &str) -> Result<bool, MicError> {
            self.devices
                .lock()
                .expect("fake mic")
                .get(device_id)
                .copied()
                .ok_or(MicError::DeviceNotFound)
        }

        fn set_muted(&self, device_id: &str, muted: bool) -> Result<(), MicError> {
            let mut devices = self.devices.lock().expect("fake mic");
            let slot = devices.get_mut(device_id).ok_or(MicError::DeviceNotFound)?;
            *slot = muted;
            Ok(())
        }
    }

    #[test]
    fn set_muted_writes_the_requested_state() {
        let mic = FakeMic::new(&[("default", false)]);
        mic.set_muted("default", true).unwrap();
        mic.set_muted("default", true).unwrap();
        assert!(mic.get_muted("default").unwrap());
        mic.set_muted("default", false).unwrap();
        assert!(!mic.get_muted("default").unwrap());
    }

    #[test]
    fn rapid_toggle_tracks_the_latest_state() {
        let mic = FakeMic::new(&[("default", false)]);
        let mut expected = false;
        for _ in 0..20 {
            expected = !expected;
            assert_eq!(toggle_mute(&mic, "default").unwrap(), expected);
            assert_eq!(mic.get_muted("default").unwrap(), expected);
        }
        assert!(!expected);
    }

    #[test]
    fn missing_device_is_unavailable_not_unmuted() {
        let mic = FakeMic::new(&[("default", true)]);
        mic.remove("default");
        assert_eq!(
            mute_status(&mic, "default").unwrap(),
            MuteStatus::Unavailable
        );
        assert_eq!(mic.get_muted("default"), Err(MicError::DeviceNotFound));
        assert_eq!(toggle_mute(&mic, "default"), Err(MicError::DeviceNotFound));
    }
}
