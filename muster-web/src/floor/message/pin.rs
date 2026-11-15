use muster_protocol::service::{EventKind, StackEvent};
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
    /// The pin `event` drops: a restart is again, a crash or a refused start
    /// fail, a passed check ok, a check out of time late, and a start ok
    /// only when the service says its run armed no readiness check. Other
    /// kinds pin nothing.
    #[must_use]
    pub(crate) fn of_event(event: &StackEvent) -> Option<Self> {
        match event.kind() {
            EventKind::Restarting => Some(Self::Again),
            EventKind::Crashed | EventKind::Blocked => Some(Self::Fail),
            EventKind::Ready => Some(Self::Ok),
            EventKind::Started if event.waits_ready() == Some(false) => Some(Self::Ok),
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
    use crate::floor::fixture::{stack_event, started_event};

    /// Each kind maps to its pin; a start pins ok only when its run armed
    /// no readiness check, and a stop, an exit or a notice pins nothing.
    #[test]
    fn each_event_kind_maps_to_its_pin() {
        let pin = |kind| Pin::of_event(&stack_event("shop", "web", kind));

        assert_eq!(pin(EventKind::Restarting), Some(Pin::Again));
        assert_eq!(pin(EventKind::Crashed), Some(Pin::Fail));
        assert_eq!(pin(EventKind::Blocked), Some(Pin::Fail));
        assert_eq!(pin(EventKind::Ready), Some(Pin::Ok));
        assert_eq!(pin(EventKind::ReadyTimeout), Some(Pin::Late));
        assert_eq!(pin(EventKind::Started), Some(Pin::Ok));
        assert_eq!(
            Pin::of_event(&started_event("shop", "web", false)),
            Some(Pin::Ok)
        );
        assert_eq!(Pin::of_event(&started_event("shop", "web", true)), None);
        for kind in [
            EventKind::Stopped,
            EventKind::Exited,
            EventKind::Notice,
            EventKind::IdleStopped,
        ] {
            assert_eq!(pin(kind), None, "{kind:?}");
        }
    }
}
