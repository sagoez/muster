use serde_json::json;

use super::{
    answer::Answer, error::CliError, lock_command::LockCommand, row::Row, row_kind::RowKind,
};
use crate::{
    adapter::{client::LockView, store::now_epoch_secs, wire::WireForm},
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
/// The JSON key of the listed locks.
const LOCKS: &str = "locks";
/// The JSON key of the lock a release named.
const NAME: &str = "name";
/// The JSON key of whether a release found the lock held.
const RELEASED: &str = "released";
/// The JSON key of who held the lock a release cleared.
const HELD_BY: &str = "held_by";

/// Runs a `muster lock` action against `store` for `project`, returning what
/// to print. Absent a subcommand, lists the locks held.
///
/// # Errors
/// Returns [`CliError`] when a name is invalid or the store cannot be read or
/// written.
pub fn lock(
    command: Option<LockCommand>,
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
) -> Result<Answer, CliError> {
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
) -> Result<Answer, CliError> {
    let locks = store.locks(project)?;
    let views: Vec<LockView> = locks.iter().map(|lock| LockView::at(lock, now)).collect();
    let rows = if locks.is_empty() {
        vec![Row::unlabeled(RowKind::Hint, EMPTY_NOTE)]
    } else {
        locks
            .iter()
            .map(|lock| Row::unlabeled(RowKind::Plain, line(lock, now)))
            .collect()
    };
    Answer::of(rows, &json!({ LOCKS: views }))
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
) -> Result<Answer, CliError> {
    let parsed =
        LockName::try_new(name).map_err(|_| CliError::InvalidLockName(name.to_string()))?;
    let broken = store.break_lock(project, &parsed)?;
    let row = match &broken {
        Some(lock) => Row::unlabeled(
            RowKind::Ok,
            format!("released '{parsed}' (held by {})", lock.owner()),
        ),
        None => Row::unlabeled(RowKind::Hint, format!("no lock '{parsed}' to release")),
    };
    let held_by = broken.as_ref().map(|lock| lock.owner().to_wire());
    Answer::of(
        vec![row],
        &json!({ NAME: parsed.to_string(), RELEASED: broken.is_some(), HELD_BY: held_by }),
    )
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
            list(&store, &root, now_epoch_secs() as u64).unwrap().rows()[0].detail(),
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

        let listed = list(&store, &root, lock.expires_at() - 90).unwrap();

        assert_eq!(
            listed.rows()[0].detail(),
            &format!(
                "migrations  claude  {} left  running 0042",
                humantime::format_duration(Duration::from_secs(90))
            )
        );
        assert_eq!(listed.json()["locks"][0]["name"], "migrations");
        assert_eq!(listed.json()["locks"][0]["expires_in_s"], 90);
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
        assert_eq!(first.rows()[0].kind(), RowKind::Ok);
        assert!(first.rows()[0].detail().contains("held by claude"));
        assert_eq!(first.json()["released"], true);
        assert!(store.locks(&root).unwrap().is_empty());
        let second = release(&store, &root, "db-reset").unwrap();
        assert_eq!(second.rows()[0].kind(), RowKind::Hint);
        assert_eq!(second.json()["released"], false);
        assert!(second.json()["held_by"].is_null());
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
