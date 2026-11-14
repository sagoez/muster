use std::{io, sync::Arc, thread};

use super::{
    events_poll::poll_events,
    logs_poll::poll_logs,
    notes_poll::{poll_notes, read_notes},
    projects_poll::{poll_projects, read_projects},
    state_access::StateAccess,
    status_poll::{poll_status, status_event},
};
use crate::{
    live::{hub_event::HubEvent, hub_handle::HubHandle, pause::Pause},
    view_deps::ViewDeps,
};

/// The projects poller's thread name.
const PROJECTS_THREAD: &str = "view-projects";
/// The status poller's thread name.
const STATUS_THREAD: &str = "view-status";
/// The events poller's thread name.
const EVENTS_THREAD: &str = "view-events";
/// The notes poller's thread name.
const NOTES_THREAD: &str = "view-notes";
/// The logs poller's thread name.
const LOGS_THREAD: &str = "view-logs";

/// Feeds `hub` from `deps`: reads the projects, the status and the notes
/// once now, so the first page to connect sees them, then starts a poller
/// thread for each source. The status poller waits on `wake`, the others on
/// `life`; each ends once the hub is gone.
///
/// # Errors
/// Returns the failure to start a thread.
pub fn start_pollers(
    deps: &ViewDeps,
    hub: &HubHandle,
    life: &Pause,
    wake: Pause,
) -> io::Result<()> {
    let mut projects = StateAccess::new(Arc::clone(deps.state()));
    let mut notes = StateAccess::new(Arc::clone(deps.state()));
    let manifests = Arc::clone(deps.manifests());
    let service = Arc::clone(deps.service());
    hub.send(HubEvent::Projects(read_projects(
        &mut projects,
        manifests.as_ref(),
    )));
    hub.send(status_event(service.as_ref()));
    for event in read_notes(&mut notes) {
        hub.send(event);
    }
    spawn(PROJECTS_THREAD, hub, life, move |hub, pause| {
        poll_projects(projects, &manifests, hub, pause);
    })?;
    let polled = Arc::clone(&service);
    spawn(STATUS_THREAD, hub, &wake, move |hub, wake| {
        poll_status(&polled, hub, wake);
    })?;
    spawn(EVENTS_THREAD, hub, life, move |hub, pause| {
        poll_events(&service, hub, pause);
    })?;
    spawn(NOTES_THREAD, hub, life, move |hub, pause| {
        poll_notes(notes, hub, pause);
    })?;
    spawn(LOGS_THREAD, hub, life, poll_logs)?;
    Ok(())
}

/// Runs `poll` on a thread named `name`, handed its own `hub` and `pause`.
///
/// # Errors
/// Returns the failure to start the thread.
fn spawn(
    name: &str,
    hub: &HubHandle,
    pause: &Pause,
    poll: impl FnOnce(&HubHandle, &Pause) + Send + 'static,
) -> io::Result<()> {
    let (hub, pause) = (hub.clone(), pause.clone());
    thread::Builder::new()
        .name(name.to_string())
        .spawn(move || poll(&hub, &pause))
        .map(drop)
}
