use std::{sync::Arc, time::Duration};

use muster_domain::config::ConfigError;

use super::state_access::StateAccess;
use crate::{
    ManifestReader,
    floor::{Health, ProjectReading, ProjectsReading},
    live::{hub_event::HubEvent, hub_handle::HubHandle, pause::Pause},
};

/// How often the registered projects and their stacks are read again.
pub const PROJECTS_EVERY: Duration = Duration::from_secs(5);

/// Every registered project with its stored stack, its icons resolved over
/// `manifests`; or, when the state cannot be read, how it stands. A failed
/// read closes the state, so the next one opens it afresh.
pub fn read_projects(state: &mut StateAccess, manifests: &dyn ManifestReader) -> ProjectsReading {
    let store = match state.store() {
        Ok(store) => store,
        Err(reading) => return reading,
    };
    let read = || -> Result<Vec<ProjectReading>, ConfigError> {
        store
            .projects()?
            .iter()
            .filter_map(|project| store.stack(project.root()).transpose())
            .map(|stack| stack.map(|stack| ProjectReading::of(&stack, manifests)))
            .collect()
    };
    read().map_or_else(
        |_| {
            state.forget();
            ProjectsReading::Trouble(Health::Unreadable)
        },
        ProjectsReading::Read,
    )
}

/// Reads the projects now, then every [`PROJECTS_EVERY`], and sends each
/// reading to `hub`, until the hub is gone.
pub fn poll_projects(
    mut state: StateAccess,
    manifests: &Arc<dyn ManifestReader>,
    hub: &HubHandle,
    pause: &Pause,
) {
    while hub.send(HubEvent::Projects(read_projects(
        &mut state,
        manifests.as_ref(),
    ))) && pause.wait(PROJECTS_EVERY)
    {}
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{
        floor::fixture::floor, icon::fake_manifests::FakeManifests, opened_state::OpenedState,
    };

    /// What a state opener answers.
    type Opened = Result<OpenedState, ConfigError>;

    /// What reading the projects finds over a state whose opener answers
    /// `opened`, and the health a floor shows for it.
    fn read_over(opened: fn() -> Opened) -> (ProjectsReading, Health) {
        let mut state = StateAccess::new(Arc::new(opened), PathBuf::from("/nowhere/muster.db"));
        let reading = read_projects(&mut state, &FakeManifests::default());
        let mut floor = floor();
        floor.read_projects(reading.clone());
        (reading, floor.health())
    }

    /// No database reads as no state, one an older or a newer muster wrote
    /// as an older or a newer schema, and one that fails to open as
    /// unreadable, each the health the floor then shows.
    #[test]
    fn no_database_and_an_older_or_newer_one_read_as_their_health() {
        for (opened, health) in [
            (
                (|| Ok(OpenedState::Absent)) as fn() -> Opened,
                Health::NoState,
            ),
            (|| Ok(OpenedState::Older), Health::OlderSchema),
            (
                || {
                    Err(ConfigError::UnsupportedSchema {
                        found: 99,
                        supported: 1,
                    })
                },
                Health::NewerSchema,
            ),
            (|| Err(ConfigError::TooManyLocks), Health::Unreadable),
        ] {
            assert_eq!(
                read_over(opened),
                (ProjectsReading::Trouble(health), health)
            );
        }
    }
}
