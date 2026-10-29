use super::{
    args::LockCommand,
    error::CliError,
    report::{Row, RowKind},
};
use crate::{
    adapter::store::now_epoch_secs,
    domain::{
        coordination::{Lock, LockName},
        port::CoordinationStore,
        value::ProjectRoot,
    },
};

/// Line printed when no lock is held in the project.
const EMPTY_NOTE: &str = "no locks held";
/// Columns between the fields of a listed lock line.
const FIELD_GAP: &str = "  ";

/// Runs a `muster lock` action against `store` for `project`, returning the
/// rows to print. Absent a subcommand, lists the locks held.
///
/// # Errors
/// Returns [`CliError`] when a name is invalid or the store cannot be read or
/// written.
pub fn lock(
    command: Option<LockCommand>,
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
) -> Result<Vec<Row>, CliError> {
    match command {
        None => list(store, project, now_epoch_secs() as u64),
        Some(LockCommand::Release { name }) => release(store, project, &name),
    }
}

/// Lists every lock held in `project` at `now` (Unix seconds) as
/// `name  owner  <left> left  note`, one per line.
///
/// # Errors
/// Returns [`CliError`] when the store cannot be read.
fn list(
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
    now: u64,
) -> Result<Vec<Row>, CliError> {
    let locks = store.locks(project)?;
    if locks.is_empty() {
        return Ok(vec![Row::unlabeled(RowKind::Hint, EMPTY_NOTE)]);
    }
    Ok(locks
        .iter()
        .map(|lock| Row::unlabeled(RowKind::Plain, line(lock, now)))
        .collect())
}

/// One listed lock at `now`.
fn line(lock: &Lock, now: u64) -> String {
    let note = lock
        .note()
        .as_ref()
        .map(|note| format!("{FIELD_GAP}{note}"))
        .unwrap_or_default();
    format!(
        "{name}{FIELD_GAP}{owner}{FIELD_GAP}{left} left{note}",
        name = lock.name(),
        owner = lock.owner(),
        left = humantime::format_duration(lock.expires_in(now)),
    )
}

/// Releases the lock `name` whoever holds it, so the human can clear one an
/// agent left behind. Releasing a lock nobody holds is not an error.
///
/// # Errors
/// Returns [`CliError::InvalidLockName`] for a malformed name, or a store
/// failure.
fn release(
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
    name: &str,
) -> Result<Vec<Row>, CliError> {
    let parsed =
        LockName::try_new(name).map_err(|_| CliError::InvalidLockName(name.to_string()))?;
    Ok(vec![match store.break_lock(project, &parsed)? {
        Some(lock) => Row::unlabeled(
            RowKind::Ok,
            format!("released '{parsed}' (held by {})", lock.owner()),
        ),
        None => Row::unlabeled(RowKind::Hint, format!("no lock '{parsed}' to release")),
    }])
}

#[cfg(test)]
mod tests {
    use std::{fs, time::Duration};

    use tempfile::TempDir;

    use super::*;
    use crate::{
        adapter::store::{
            SqliteStore,
            test_support::{fresh_store, spec},
        },
        domain::{
            coordination::{Author, LockNote, LockTtl},
            port::StackStore,
        },
    };

    /// A store with one registered project, and the scratch folder holding
    /// both.
    fn fixture() -> (SqliteStore, ProjectRoot, TempDir) {
        let (store, scratch) = fresh_store();
        let dir = scratch.path().join("shop");
        fs::create_dir_all(&dir).unwrap();
        let root = ProjectRoot::try_new(dir.canonicalize().unwrap()).unwrap();
        store
            .add_process(&root, &spec("web", "true"), &Author::human())
            .unwrap();
        (store, root, scratch)
    }

    /// A held lock lists its holder, what it has left and its note.
    #[test]
    fn list_shows_holder_lease_and_note() {
        let (store, root, _scratch) = fixture();
        assert_eq!(
            list(&store, &root, now_epoch_secs() as u64).unwrap()[0].detail(),
            EMPTY_NOTE
        );
        store
            .acquire_lock(
                &root,
                &Author::agent("claude"),
                &LockName::try_new("migrations").unwrap(),
                LockTtl::DEFAULT,
                Some(&LockNote::try_new("running 0042").unwrap()),
            )
            .unwrap();
        let lock = store.locks(&root).unwrap().remove(0);

        let rows = list(&store, &root, lock.expires_at() - 90).unwrap();

        assert_eq!(
            rows[0].detail(),
            &format!(
                "migrations  claude  {} left  running 0042",
                humantime::format_duration(Duration::from_secs(90))
            )
        );
    }

    /// The human releases an agent's lock; a second release says none is held.
    #[test]
    fn release_clears_anyones_lock() {
        let (store, root, _scratch) = fixture();
        let name = LockName::try_new("db-reset").unwrap();
        store
            .acquire_lock(
                &root,
                &Author::agent("claude"),
                &name,
                LockTtl::DEFAULT,
                None,
            )
            .unwrap();

        let first = release(&store, &root, "db-reset").unwrap();
        assert_eq!(first[0].kind(), RowKind::Ok);
        assert!(first[0].detail().contains("held by claude"));
        assert!(store.locks(&root).unwrap().is_empty());
        assert_eq!(
            release(&store, &root, "db-reset").unwrap()[0].kind(),
            RowKind::Hint
        );
    }

    /// A malformed name is refused before the store is touched.
    #[test]
    fn a_malformed_name_is_refused() {
        let (store, root, _scratch) = fixture();
        assert!(matches!(
            release(&store, &root, "Not A Lock"),
            Err(CliError::InvalidLockName(_))
        ));
    }
}
