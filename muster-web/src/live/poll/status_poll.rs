use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

use muster_client::ServiceAccess;
use muster_protocol::service::ServiceRequest;

use crate::{
    floor::StatusReading,
    live::{hub_event::HubEvent, hub_handle::HubHandle, pause::Pause},
};

/// How often every loaded project's status is asked for, unless an event
/// asks for it sooner.
pub const STATUS_EVERY: Duration = Duration::from_secs(1);

/// What asking `service` for every loaded project's status found, stamped
/// with when the answer came. It asks only a service that runs, never
/// starting one, and loads no project.
pub fn status_event(service: &dyn ServiceAccess) -> HubEvent {
    let asked = service.ask_if_running(&ServiceRequest::Ps { project: None });
    HubEvent::Status {
        reading: StatusReading::from(asked),
        now: Instant::now(),
        wall: SystemTime::now(),
    }
}

/// Asks for status every [`STATUS_EVERY`], or at once when the hub wakes
/// `wake`, and sends each answer to `hub`, until the hub is gone.
pub fn poll_status(service: &Arc<dyn ServiceAccess>, hub: &HubHandle, wake: &Pause) {
    while wake.wait(STATUS_EVERY) && hub.send(status_event(service.as_ref())) {}
}

#[cfg(test)]
mod tests {
    use muster_client::fake_service::FakeService;

    use super::*;

    /// An absent service reads as absent, asked only with every loaded
    /// project's `Ps`, and never connected to, so never started.
    #[test]
    fn an_absent_service_reads_as_absent_and_is_never_started() {
        let service = FakeService::default();

        let event = status_event(&service);

        let HubEvent::Status { reading, .. } = event else {
            panic!("not a status");
        };
        assert_eq!(reading, StatusReading::Absent);
        assert_eq!(service.asked(), vec![ServiceRequest::Ps { project: None }]);
        assert_eq!(service.connects(), 0);
    }
}
