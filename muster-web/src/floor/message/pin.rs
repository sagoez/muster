use muster_protocol::service::EventKind;
use serde::Serialize;

/// What drops on a process's face when the service logs a change of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Pin {
    /// It came into Running.
    Ok,
    /// It is waiting to start again.
    Again,
    /// It failed, or could not start.
    Fail,
    /// It runs, but its readiness check ran out of time.
    Late,
}

impl Pin {
    /// The pin an event of `kind` drops on a process whose runs wait on a
    /// readiness check when `checked`: a restart is again, a crash or a
    /// refused start fail, a passed check ok, a check out of time late, and
    /// a start ok only with no check to pass. Other kinds pin nothing.
    #[must_use]
    pub(crate) fn of_event(kind: EventKind, checked: bool) -> Option<Self> {
        match kind {
            EventKind::Restarting => Some(Self::Again),
            EventKind::Crashed | EventKind::Blocked => Some(Self::Fail),
            EventKind::Ready => Some(Self::Ok),
            EventKind::Started if !checked => Some(Self::Ok),
            EventKind::ReadyTimeout => Some(Self::Late),
            EventKind::Started
            | EventKind::Exited
            | EventKind::Stopped
            | EventKind::Notice
            | EventKind::IdleStopped => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each kind maps to its pin; a start pins ok only with no check, and a
    /// stop, an exit or a notice pins nothing.
    #[test]
    fn each_event_kind_maps_to_its_pin() {
        let unchecked = |kind| Pin::of_event(kind, false);

        assert_eq!(unchecked(EventKind::Restarting), Some(Pin::Again));
        assert_eq!(unchecked(EventKind::Crashed), Some(Pin::Fail));
        assert_eq!(unchecked(EventKind::Blocked), Some(Pin::Fail));
        assert_eq!(unchecked(EventKind::Ready), Some(Pin::Ok));
        assert_eq!(unchecked(EventKind::ReadyTimeout), Some(Pin::Late));
        assert_eq!(unchecked(EventKind::Started), Some(Pin::Ok));
        assert_eq!(Pin::of_event(EventKind::Started, true), None);
        assert_eq!(Pin::of_event(EventKind::Ready, true), Some(Pin::Ok));
        for kind in [
            EventKind::Stopped,
            EventKind::Exited,
            EventKind::Notice,
            EventKind::IdleStopped,
        ] {
            assert_eq!(unchecked(kind), None, "{kind:?}");
        }
    }
}
