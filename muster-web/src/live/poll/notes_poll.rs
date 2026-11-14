use std::time::Duration;

use super::state_access::StateAccess;
use crate::live::{hub_event::HubEvent, hub_handle::HubHandle, pause::Pause};

/// How often every project's notes are read again.
pub const NOTES_EVERY: Duration = Duration::from_secs(2);

/// Every registered project's notes, bodies included, one event a project;
/// none while the state cannot be read. A project whose notes fail to read
/// is left out this time, and a failure to list the projects closes the
/// state, so the next read opens it afresh.
pub fn read_notes(state: &mut StateAccess) -> Vec<HubEvent> {
    let Ok(store) = state.store() else {
        return Vec::new();
    };
    let Ok(projects) = store.projects() else {
        state.forget();
        return Vec::new();
    };
    projects
        .iter()
        .filter_map(|project| {
            let notes = store.scratchpads(project.root()).ok()?;
            Some(HubEvent::Notes {
                root: project.root().clone(),
                notes,
            })
        })
        .collect()
}

/// Reads the notes every [`NOTES_EVERY`] and sends them to `hub`, until the
/// hub is gone.
pub fn poll_notes(mut state: StateAccess, hub: &HubHandle, pause: &Pause) {
    while pause.wait(NOTES_EVERY) {
        for event in read_notes(&mut state) {
            if !hub.send(event) {
                return;
            }
        }
    }
}
