use std::path::Path;

use serde_json::json;

use super::{
    answer::Answer, coordination_command::CoordinationCommand, error::CliError, row::Row,
    row_kind::RowKind, text_file::write_privately,
};
use crate::{
    adapter::{
        bounded_read::read_bounded,
        exchange::{CoordinationSnapshot, SNAPSHOT_VERSION},
    },
    domain::{port::CoordinationStore, value::ProjectRoot},
};

/// Largest coordination snapshot import reads: room for many full notes,
/// never a whole disk.
const MAX_SNAPSHOT_BYTES: u64 = 16 * 1024 * 1024;

/// The JSON key of the snapshot file.
const FILE: &str = "file";
/// The JSON key of how many scratchpads moved.
const SCRATCHPADS: &str = "scratchpads";
/// The JSON key of how many todos moved.
const TODOS: &str = "todos";
/// The JSON key of how many values moved.
const VALUES: &str = "values";

/// Runs a `muster coordination` action against `store` for `project`, returning
/// what to print.
///
/// # Errors
/// Returns [`CliError`] when the file cannot be read or written, the snapshot is
/// not valid YAML, its version is unsupported, or the store fails.
pub fn coordination(
    command: CoordinationCommand,
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
) -> Result<Answer, CliError> {
    match command {
        CoordinationCommand::Export { file } => export(store, project, &file),
        CoordinationCommand::Import { file } => import(store, project, &file),
    }
}

/// What moving `snapshot` through `file` answers: `summary` for a person,
/// the file and the counts for `--json`.
///
/// # Errors
/// Returns [`CliError::Output`] when the JSON cannot be rendered.
fn moved(
    snapshot: &CoordinationSnapshot,
    file: &Path,
    summary: String,
) -> Result<Answer, CliError> {
    Answer::of(
        vec![Row::unlabeled(RowKind::Ok, summary)],
        &json!({
            FILE: file,
            SCRATCHPADS: snapshot.scratchpads().len(),
            TODOS: snapshot.todos().len(),
            VALUES: snapshot.values().len(),
        }),
    )
}

/// Writes this project's coordination state to `file` as YAML, readable by
/// its owner alone, since notes and values may hold tokens.
///
/// # Errors
/// Returns [`CliError`] when the store cannot be read or the file written.
fn export(
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
    file: &Path,
) -> Result<Answer, CliError> {
    let snapshot = CoordinationSnapshot::builder()
        .scratchpads(store.scratchpads(project)?)
        .todos(store.todos(project)?)
        .values(store.values(project)?)
        .build();
    let summary = format!(
        "exported {} scratchpads, {} todos, {} values to {}",
        snapshot.scratchpads().len(),
        snapshot.todos().len(),
        snapshot.values().len(),
        file.display(),
    );
    let text = serde_yaml_ng::to_string(&snapshot).map_err(|error| CliError::SnapshotFile {
        path: file.to_path_buf(),
        reason: error.to_string(),
    })?;
    write_privately(file, &text).map_err(|source| CliError::WriteFile {
        path: file.to_path_buf(),
        source,
    })?;
    moved(&snapshot, file, summary)
}

/// Restores entries from `file` into this project, preserving ids, authors, and
/// timestamps. Entries merge: one sharing a key or id replaces what is there,
/// and entries absent from the file are left alone. Only a regular file of at
/// most [`MAX_SNAPSHOT_BYTES`] is read.
///
/// # Errors
/// Returns [`CliError`] when the file cannot be read, is not a regular file
/// or is too large, is not a valid snapshot,
/// carries an unsupported version, or the store cannot be written.
fn import(
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
    file: &Path,
) -> Result<Answer, CliError> {
    let raw = read_bounded(file, MAX_SNAPSHOT_BYTES).map_err(|source| CliError::ReadFile {
        path: file.to_path_buf(),
        source,
    })?;
    let invalid = |reason: String| CliError::SnapshotFile {
        path: file.to_path_buf(),
        reason,
    };
    let snapshot: CoordinationSnapshot =
        serde_yaml_ng::from_str(&raw).map_err(|error| invalid(error.to_string()))?;
    if snapshot.version() != SNAPSHOT_VERSION {
        return Err(invalid(format!(
            "version {} is not the version {SNAPSHOT_VERSION} this muster reads",
            snapshot.version()
        )));
    }
    store.import(
        project,
        snapshot.scratchpads(),
        snapshot.todos(),
        snapshot.values(),
    )?;
    let summary = format!(
        "imported {} scratchpads, {} todos, {} values from {}",
        snapshot.scratchpads().len(),
        snapshot.todos().len(),
        snapshot.values().len(),
        file.display(),
    );
    moved(&snapshot, file, summary)
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, File, Permissions},
        os::unix::fs::PermissionsExt,
    };

    use tempfile::TempDir;

    use super::*;
    use crate::{
        adapter::{
            store::{
                SqliteStore,
                test_support::{fresh_store, spec},
            },
            test_fifo::make_fifo,
        },
        domain::{coordination::Author, port::StackStore},
    };

    /// The mode bits that say who may read a file.
    const PERMISSION_BITS: u32 = 0o777;
    /// The mode `export` gives the snapshot.
    const OWNER_ONLY: u32 = 0o600;
    /// A mode anyone may read.
    const WORLD_READABLE: u32 = 0o644;

    /// A store with the project `shop` registered in `scratch`, and its root.
    fn project() -> (SqliteStore, TempDir, ProjectRoot) {
        let (store, scratch) = fresh_store();
        let dir = scratch.path().join("shop");
        fs::create_dir_all(&dir).unwrap();
        let root = ProjectRoot::try_new(dir.canonicalize().unwrap()).unwrap();
        store
            .add_process(&root, &spec("web", "true"), &Author::human())
            .unwrap();
        (store, scratch, root)
    }

    /// An import refuses a FIFO without blocking on it, and a file past the
    /// cap without reading it.
    #[test]
    fn an_import_reads_only_a_bounded_regular_file() {
        let (store, scratch, root) = project();
        let fifo = scratch.path().join("snapshot.fifo");
        make_fifo(&fifo);
        let huge = scratch.path().join("huge.yml");
        File::create(&huge)
            .unwrap()
            .set_len(MAX_SNAPSHOT_BYTES + 1)
            .unwrap();

        for file in [fifo, huge] {
            let refused = coordination(
                CoordinationCommand::Import { file: file.clone() },
                &store,
                &root,
            )
            .unwrap_err();
            assert!(
                matches!(&refused, CliError::ReadFile { path, .. } if path == &file),
                "{refused}"
            );
        }
    }

    /// An export is readable by its owner alone, even over a file anyone
    /// could read.
    #[test]
    fn an_export_is_written_owner_only() {
        let (store, scratch, root) = project();
        let file = scratch.path().join("snapshot.yml");
        fs::write(&file, "old").unwrap();
        fs::set_permissions(&file, Permissions::from_mode(WORLD_READABLE)).unwrap();

        coordination(
            CoordinationCommand::Export { file: file.clone() },
            &store,
            &root,
        )
        .unwrap();

        let mode = fs::metadata(&file).unwrap().permissions().mode() & PERMISSION_BITS;
        assert_eq!(mode, OWNER_ONLY);
        coordination(CoordinationCommand::Import { file }, &store, &root).unwrap();
    }
}
