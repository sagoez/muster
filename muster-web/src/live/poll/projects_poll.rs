use std::{sync::Arc, time::Duration};

use super::state_access::StateAccess;
use crate::{
    ManifestReader,
    floor::{Health, ProjectReading, ProjectsReading},
    live::{hub_event::HubEvent, hub_handle::HubHandle, pause::Pause},
};

/// How often the registered projects and their stacks are read again.
pub const PROJECTS_EVERY: Duration = Duration::from_secs(5);

/// Every registered project with its stored stack, its icons resolved over
/// `manifests`; a project whose stack does not read is marked so, and the
/// others read on. When the state cannot be read, how it stands; a failure
/// to list the projects closes the state, so the next read opens it afresh.
pub fn read_projects(state: &mut StateAccess, manifests: &dyn ManifestReader) -> ProjectsReading {
    let store = match state.store() {
        Ok(store) => store,
        Err(reading) => return reading,
    };
    let Ok(projects) = store.projects() else {
        state.forget();
        return ProjectsReading::Trouble(Health::Unreadable);
    };
    let readings = projects
        .iter()
        .filter_map(|project| match store.stack(project.root()) {
            Ok(stack) => stack.map(|stack| ProjectReading::of(&stack, manifests)),
            Err(_) => Some(ProjectReading::unread(project)),
        })
        .collect();
    ProjectsReading::Read(readings)
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
    use std::{fs, path::PathBuf};

    use muster_domain::{
        config::ConfigError,
        coordination::Author,
        port::{LocalState, StackStore},
        value::ProjectRoot,
    };
    use muster_store::test_support::{database, fresh_store, spec};
    use rusqlite::{Connection, params};

    use super::*;
    use crate::{
        floor::fixture::floor, icon::fake_manifests::FakeManifests, opened_state::OpenedState,
    };

    /// A restart policy no muster writes, which the store refuses to read.
    const BOGUS_RESTART: &str = "bogus";

    /// One project's stack whose process row does not read is read as that
    /// project unreadable, with no processes, and every other project reads
    /// on with its stack.
    #[test]
    fn one_stack_that_does_not_read_leaves_the_others() {
        let (store, scratch) = fresh_store();
        let root = |name: &str| {
            let dir = scratch.path().join(name);
            fs::create_dir_all(&dir).unwrap();
            ProjectRoot::try_new(dir).unwrap()
        };
        let (shop, blog) = (root("shop"), root("blog"));
        for project in [&shop, &blog] {
            store
                .add_process(project, &spec("web", "cargo run"), &Author::human())
                .unwrap();
        }
        Connection::open(database(&scratch))
            .unwrap()
            .execute(
                "UPDATE processes SET restart = ?1 \
                 WHERE project_id = (SELECT id FROM projects WHERE root = ?2)",
                params![BOGUS_RESTART, blog.as_ref().to_string_lossy()],
            )
            .unwrap();
        let store: Arc<dyn LocalState> = Arc::new(store);
        let mut state = StateAccess::new(
            Arc::new(move || Ok(OpenedState::Current(Arc::clone(&store)))),
            database(&scratch),
        );

        let ProjectsReading::Read(projects) = read_projects(&mut state, &FakeManifests::default())
        else {
            panic!("the projects did not read");
        };

        let read = |root: &ProjectRoot| {
            projects
                .iter()
                .find(|reading| reading.project().root() == root)
                .unwrap()
        };
        assert_eq!(projects.len(), 2);
        assert!(!*read(&shop).unreadable());
        assert_eq!(read(&shop).processes().len(), 1);
        assert!(*read(&blog).unreadable());
        assert!(read(&blog).processes().is_empty());
    }

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
