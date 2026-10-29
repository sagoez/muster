use std::{fs, path::Path};

use super::{
    coordination_command::CoordinationCommand, error::CliError, row::Row, row_kind::RowKind,
    text_file::write_atomically,
};
use crate::{
    adapter::exchange::{CoordinationSnapshot, SNAPSHOT_VERSION},
    domain::{port::CoordinationStore, value::ProjectRoot},
};

/// Runs a `muster coordination` action against `store` for `project`, returning
/// the rows to print.
///
/// # Errors
/// Returns [`CliError`] when the file cannot be read or written, the snapshot is
/// not valid YAML, its version is unsupported, or the store fails.
pub fn coordination(
    command: CoordinationCommand,
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
) -> Result<Vec<Row>, CliError> {
    match command {
        CoordinationCommand::Export { file } => export(store, project, &file),
        CoordinationCommand::Import { file } => import(store, project, &file),
    }
}

/// Writes this project's coordination state to `file` as YAML.
///
/// # Errors
/// Returns [`CliError`] when the store cannot be read or the file written.
fn export(
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
    file: &Path,
) -> Result<Vec<Row>, CliError> {
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
    write_atomically(file, &text).map_err(|source| CliError::WriteFile {
        path: file.to_path_buf(),
        source,
    })?;
    Ok(vec![Row::unlabeled(RowKind::Ok, summary)])
}

/// Restores entries from `file` into this project, preserving ids, authors, and
/// timestamps. Entries merge: one sharing a key or id replaces what is there,
/// and entries absent from the file are left alone.
///
/// # Errors
/// Returns [`CliError`] when the file cannot be read, is not a valid snapshot,
/// carries an unsupported version, or the store cannot be written.
fn import(
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
    file: &Path,
) -> Result<Vec<Row>, CliError> {
    let raw = fs::read_to_string(file).map_err(|source| CliError::ReadFile {
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
    Ok(vec![Row::unlabeled(
        RowKind::Ok,
        format!(
            "imported {} scratchpads, {} todos, {} values from {}",
            snapshot.scratchpads().len(),
            snapshot.todos().len(),
            snapshot.values().len(),
            file.display(),
        ),
    )])
}
