use std::{sync::Arc, time::Duration};

use muster_domain::config::ConfigError;

use super::state_access::StateAccess;
use crate::{
    ManifestReader,
    floor::{ProjectReading, ProjectsReading, StateTrouble},
    live::{hub_event::HubEvent, hub_handle::HubHandle, pause::Pause},
};

/// How often the registered projects and their stacks are read again.
pub const PROJECTS_EVERY: Duration = Duration::from_secs(5);

/// Every registered project with its stored stack, its icons resolved over
/// `manifests`; a project whose stack does not read is marked so, and the
/// others read on. When the state cannot be read, how it stands. A failure
/// to list the projects closes the state, so the next read opens it afresh
/// and finds how it stands, and so do stacks that fail for [another
/// reason](must_reopen) than a row of their own.
pub fn read_projects(state: &mut StateAccess, manifests: &dyn ManifestReader) -> ProjectsReading {
    let store = match state.store() {
        Ok(store) => store,
        Err(reading) => return reading,
    };
    let Ok(projects) = store.projects() else {
        state.forget();
        return ProjectsReading::Trouble(StateTrouble::Unreadable);
    };
    let mut readings = Vec::new();
    let mut failures = Vec::new();
    for project in &projects {
        match store.stack(project.root()) {
            Ok(stack) => {
                readings.extend(stack.map(|stack| ProjectReading::of(&stack, manifests)));
            },
            Err(error) => {
                failures.push(error);
                readings.push(ProjectReading::unread(project));
            },
        }
    }
    if must_reopen(&failures, projects.len()) {
        state.forget();
    }
    ProjectsReading::Read(readings)
}

/// Whether stacks that failed to read with `failures`, of `projects` read,
/// say the state itself may have changed under its open connection, as a
/// newer muster migrating it in place would: every stack failed, or one
/// failed with anything but a row it holds that does not read.
fn must_reopen(failures: &[ConfigError], projects: usize) -> bool {
    let all_failed = !failures.is_empty() && failures.len() == projects;
    all_failed
        || failures
            .iter()
            .any(|failure| !matches!(failure, ConfigError::CorruptRow { .. }))
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
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicUsize, Ordering},
    };

    use muster_domain::{
        config::ConfigError,
        coordination::Author,
        port::{LocalState, StackStore},
        value::ProjectRoot,
    };
    use muster_store::{
        SqliteStore,
        test_support::{corrupt_processes, database, fresh_store, spec},
    };

    use super::*;
    use crate::{
        floor::{Health, fixture::floor},
        icon::fake_manifests::FakeManifests,
        opened_state::OpenedState,
    };

    /// Registers the project in the folder `name` under `scratch`, holding a
    /// `web`, in `store`, and returns its folder.
    fn add_web(store: &SqliteStore, scratch: &Path, name: &str) -> ProjectRoot {
        let dir = scratch.join(name);
        fs::create_dir_all(&dir).unwrap();
        let root = ProjectRoot::try_new(dir).unwrap();
        store
            .add_process(&root, &spec("web", "cargo run"), &Author::human())
            .unwrap();
        root
    }

    /// The state over the file `database`, each open handing out `store`,
    /// and how many times it was opened.
    fn counted(store: SqliteStore, database: PathBuf) -> (StateAccess, Arc<AtomicUsize>) {
        let opens = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&opens);
        let store: Arc<dyn LocalState> = Arc::new(store);
        let state = StateAccess::new(
            Arc::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(OpenedState::Current(Arc::clone(&store)))
            }),
            database,
        );
        (state, opens)
    }

    /// One project's stack whose process row does not read is read as that
    /// project unreadable, with no processes, and every other project reads
    /// on with its stack; the state stays open, since only that row is wrong.
    #[test]
    fn one_stack_that_does_not_read_leaves_the_others() {
        let (store, scratch) = fresh_store();
        let shop = add_web(&store, scratch.path(), "shop");
        let blog = add_web(&store, scratch.path(), "blog");
        corrupt_processes(&database(&scratch), &blog);
        let (mut state, opens) = counted(store, database(&scratch));

        let ProjectsReading::Read(projects) = read_projects(&mut state, &FakeManifests::default())
        else {
            panic!("the projects did not read");
        };
        read_projects(&mut state, &FakeManifests::default());

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
        assert_eq!(opens.load(Ordering::SeqCst), 1);
    }

    /// When no project's stack reads, the state is opened afresh for the
    /// next read, which then finds how it stands, as after a newer muster
    /// migrated it in place.
    #[test]
    fn no_stack_that_reads_opens_the_state_afresh() {
        let (store, scratch) = fresh_store();
        let shop = add_web(&store, scratch.path(), "shop");
        corrupt_processes(&database(&scratch), &shop);
        let (mut state, opens) = counted(store, database(&scratch));

        let first = read_projects(&mut state, &FakeManifests::default());
        read_projects(&mut state, &FakeManifests::default());

        let ProjectsReading::Read(projects) = first else {
            panic!("the projects did not read");
        };
        assert!(*projects[0].unreadable());
        assert_eq!(opens.load(Ordering::SeqCst), 2);
    }

    /// Stacks that fail only on rows of their own leave the state open while
    /// another reads; every stack failing, or any failing otherwise, opens it
    /// afresh.
    #[test]
    fn only_a_failure_beyond_a_row_reopens_the_state() {
        let row = || ConfigError::CorruptRow {
            what: "stop grace".to_string(),
            reason: "too long".to_string(),
        };

        assert!(!must_reopen(&[], 0));
        assert!(!must_reopen(&[], 2));
        assert!(!must_reopen(&[row()], 2));
        assert!(must_reopen(&[row()], 1));
        assert!(must_reopen(&[row(), row()], 2));
        assert!(must_reopen(&[ConfigError::TooManyLocks], 2));
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
        for (opened, trouble) in [
            (
                (|| Ok(OpenedState::Absent)) as fn() -> Opened,
                StateTrouble::NoState,
            ),
            (|| Ok(OpenedState::Older), StateTrouble::OlderSchema),
            (
                || {
                    Err(ConfigError::UnsupportedSchema {
                        found: 99,
                        supported: 1,
                    })
                },
                StateTrouble::NewerSchema,
            ),
            (|| Err(ConfigError::TooManyLocks), StateTrouble::Unreadable),
        ] {
            assert_eq!(
                read_over(opened),
                (ProjectsReading::Trouble(trouble), Health::from(trouble))
            );
        }
    }
}
