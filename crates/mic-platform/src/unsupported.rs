use mic_core::{MicError, MicrophoneController, MicrophoneDevice};

pub struct UnsupportedController;

impl MicrophoneController for UnsupportedController {
    fn list_devices(&self) -> Result<Vec<MicrophoneDevice>, MicError> {
        Err(MicError::Unsupported)
    }

    fn get_muted(&self, _device_id: &str) -> Result<bool, MicError> {
        Err(MicError::Unsupported)
    }

    fn set_muted(&self, _device_id: &str, _muted: bool) -> Result<(), MicError> {
        Err(MicError::Unsupported)
    }
}
