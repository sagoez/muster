use std::{io, sync::Arc, thread};

use super::{
    events_poll::poll_events, logs_poll::poll_logs, notes_poll::poll_notes,
    projects_poll::poll_projects, state_access::StateAccess, status_poll::poll_status,
};
use crate::{
    live::{hub_handle::HubHandle, pause::Pause, pauses::Pauses},
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

/// Feeds `hub` from `deps`: starts a poller thread for each source, each
/// reading at once, so nothing slow holds up the caller and a page that
/// connects first sees the floor loading. Each waits on its own of
/// `pauses`, so every poller but the projects' parks while no page
/// streams; each ends once the hub is gone.
///
/// # Errors
/// Returns the failure to start a thread.
pub fn start_pollers(deps: &ViewDeps, hub: &HubHandle, pauses: &Pauses) -> io::Result<()> {
    let projects = StateAccess::new(Arc::clone(deps.state()), deps.database().clone());
    let notes = StateAccess::new(Arc::clone(deps.state()), deps.database().clone());
    let manifests = Arc::clone(deps.manifests());
    let service = Arc::clone(deps.service());
    spawn(PROJECTS_THREAD, hub, pauses.life(), move |hub, pause| {
        poll_projects(projects, &manifests, hub, pause);
    })?;
    let polled = Arc::clone(&service);
    spawn(STATUS_THREAD, hub, pauses.status(), move |hub, wake| {
        poll_status(&polled, hub, wake);
    })?;
    spawn(EVENTS_THREAD, hub, pauses.events(), move |hub, pause| {
        poll_events(&service, hub, pause);
    })?;
    spawn(NOTES_THREAD, hub, pauses.notes(), move |hub, pause| {
        poll_notes(notes, hub, pause);
    })?;
    spawn(LOGS_THREAD, hub, pauses.logs(), poll_logs)?;
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
