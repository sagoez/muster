use std::{sync::Arc, time::Duration};

use muster_domain::config::ConfigError;

use super::state_access::StateAccess;
use crate::{
    ManifestReader,
    floor::{ProjectReading, ProjectsReading},
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
            ProjectsReading::Unreadable
        },
        ProjectsReading::Read,
    )
}

/// Reads the projects every [`PROJECTS_EVERY`] and sends each reading to
/// `hub`, until the hub is gone.
pub fn poll_projects(
    mut state: StateAccess,
    manifests: &Arc<dyn ManifestReader>,
    hub: &HubHandle,
    pause: &Pause,
) {
    while pause.wait(PROJECTS_EVERY)
        && hub.send(HubEvent::Projects(read_projects(
            &mut state,
            manifests.as_ref(),
        )))
    {}
}

#[cfg(test)]
mod tests {
    use muster_domain::port::LocalState;

    use super::*;
    use crate::{
        floor::{Health, fixture::floor},
        icon::fake_manifests::FakeManifests,
    };

    /// What a state opener answers.
    type Opened = Result<Option<Arc<dyn LocalState>>, ConfigError>;

    /// What reading the projects finds over a state whose opener answers
    /// `opened`, and the health a floor shows for it.
    fn read_over(opened: fn() -> Opened) -> (ProjectsReading, Health) {
        let mut state = StateAccess::new(Arc::new(opened));
        let reading = read_projects(&mut state, &FakeManifests::default());
        let mut floor = floor();
        floor.projects(reading.clone());
        (reading, floor.health())
    }

    /// No database reads as no state, one a newer muster wrote as a newer
    /// schema, and one that fails to open as unreadable, each the health
    /// the floor then shows.
    #[test]
    fn no_database_and_a_newer_one_read_as_their_health() {
        assert_eq!(
            read_over(|| Ok(None)),
            (ProjectsReading::NoState, Health::NoState)
        );
        assert_eq!(
            read_over(|| Err(ConfigError::UnsupportedSchema {
                found: 99,
                supported: 1,
            })),
            (ProjectsReading::NewerSchema, Health::NewerSchema)
        );
        assert_eq!(
            read_over(|| Err(ConfigError::TooManyLocks)),
            (ProjectsReading::Unreadable, Health::Unreadable)
        );
    }
}
