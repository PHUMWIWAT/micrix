use crate::{MicError, MuteStatus};

/// The three labels the shell shows. A failure is unavailable, never unmuted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusLabel {
    Muted,
    Unmuted,
    Unavailable,
}

impl StatusLabel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Muted => "Muted",
            Self::Unmuted => "Unmuted",
            Self::Unavailable => "Unavailable",
        }
    }

    pub fn from_mute(status: Result<MuteStatus, MicError>) -> Self {
        match status {
            Ok(MuteStatus::Muted) => Self::Muted,
            Ok(MuteStatus::Unmuted) => Self::Unmuted,
            Ok(MuteStatus::Unavailable) | Err(_) => Self::Unavailable,
        }
    }

    fn toast_text(self) -> Option<&'static str> {
        match self {
            Self::Muted => Some("Microphone muted"),
            Self::Unmuted => Some("Microphone unmuted"),
            Self::Unavailable => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hold {
    pub shortcut: String,
    pub restore_to_unmuted: bool,
}

/// Previous mute label and the one hold-to-mute slot.
/// The first observation stores the label and does not toast.
#[derive(Debug, Default)]
pub struct MuteSession {
    previous: Option<StatusLabel>,
    hold: Option<Hold>,
}

impl MuteSession {
    pub fn new() -> Self {
        Self::default()
    }

    /// `Some` only when a later observation changes to muted or unmuted.
    pub fn observe(&mut self, label: StatusLabel) -> Option<&'static str> {
        let first = self.previous.is_none();
        let changed = self.previous != Some(label);
        self.previous = Some(label);
        if changed && !first {
            label.toast_text()
        } else {
            None
        }
    }

    pub fn can_begin_hold(&self) -> bool {
        self.hold.is_none()
    }

    /// Records a hold after mute has already succeeded. `was_muted` is the state before this press.
    pub fn commit_hold(&mut self, shortcut: String, was_muted: bool) {
        if self.hold.is_some() {
            return;
        }
        self.hold = Some(Hold {
            shortcut,
            restore_to_unmuted: !was_muted,
        });
    }

    pub fn hold(&self) -> Option<&Hold> {
        self.hold.as_ref()
    }

    /// Drops the slot when `matches` is true. The bool is whether release should unmute.
    pub fn end_hold(&mut self, matches: bool) -> Option<bool> {
        if !matches || self.hold.is_none() {
            return None;
        }
        self.hold.take().map(|hold| hold.restore_to_unmuted)
    }

    /// Exit and a real close drop the slot. `Some` means a hold was active.
    pub fn release(&mut self) -> Option<bool> {
        self.hold.take().map(|hold| hold.restore_to_unmuted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_is_unavailable() {
        assert_eq!(
            StatusLabel::from_mute(Ok(MuteStatus::Muted)),
            StatusLabel::Muted
        );
        assert_eq!(
            StatusLabel::from_mute(Ok(MuteStatus::Unmuted)),
            StatusLabel::Unmuted
        );
        assert_eq!(
            StatusLabel::from_mute(Ok(MuteStatus::Unavailable)),
            StatusLabel::Unavailable
        );
        assert_eq!(
            StatusLabel::from_mute(Err(MicError::DeviceNotFound)),
            StatusLabel::Unavailable
        );
        assert_eq!(
            StatusLabel::from_mute(Err(MicError::Platform("failed".into()))),
            StatusLabel::Unavailable
        );
        assert_eq!(StatusLabel::Unavailable.as_str(), "Unavailable");
    }

    #[test]
    fn first_observation_does_not_toast() {
        let mut session = MuteSession::new();
        assert_eq!(session.observe(StatusLabel::Muted), None);
        assert_eq!(session.observe(StatusLabel::Muted), None);
        assert_eq!(
            session.observe(StatusLabel::Unmuted),
            Some("Microphone unmuted")
        );
        assert_eq!(session.observe(StatusLabel::Unavailable), None);
        assert_eq!(
            session.observe(StatusLabel::Muted),
            Some("Microphone muted")
        );
    }

    #[test]
    fn hold_restores_only_when_this_press_muted() {
        let mut session = MuteSession::new();
        assert!(session.can_begin_hold());
        session.commit_hold("Ctrl+J".into(), false);
        assert!(!session.can_begin_hold());
        session.commit_hold("F9".into(), true);
        assert_eq!(session.hold().unwrap().shortcut, "Ctrl+J");
        assert_eq!(session.end_hold(false), None);
        assert!(session.hold().is_some());
        assert_eq!(session.end_hold(true), Some(true));
        assert!(session.can_begin_hold());

        session.commit_hold("F9".into(), true);
        assert_eq!(session.release(), Some(false));
        assert!(session.release().is_none());
    }
}
