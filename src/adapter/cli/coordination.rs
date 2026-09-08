use std::{fs, path::Path};

use serde::{Deserialize, Serialize};

use super::{
    args::CoordinationCommand,
    error::CliError,
    report::{Row, RowKind},
};
use crate::{
    adapter::config::write_config,
    domain::{
        config::ConfigError,
        coordination::{KeyValue, Scratchpad, Todo},
        port::CoordinationStore,
    },
};

/// Current snapshot schema version, checked on import so a file from a newer
/// muster is refused rather than silently half-read.
const SNAPSHOT_VERSION: u8 = 1;

/// A portable snapshot of one project's coordination state. SQLite is the
/// operational store; this YAML form is the interchange format - a backup, a
/// file to commit alongside a repo, or a way to move state between machines.
#[derive(Serialize, Deserialize)]
struct CoordinationSnapshot {
    version: u8,
    scratchpads: Vec<Scratchpad>,
    todos: Vec<Todo>,
    values: Vec<KeyValue>,
}

/// Runs a `muster coordination` action against `store` for `project`, returning
/// the rows to print.
///
/// # Errors
/// Returns [`CliError`] when the file cannot be read or written, the snapshot is
/// not valid YAML, its version is unsupported, or the store fails.
pub fn coordination(
    command: CoordinationCommand,
    store: &dyn CoordinationStore,
    project: &Path,
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
    project: &Path,
    file: &Path,
) -> Result<Vec<Row>, CliError> {
    let snapshot = CoordinationSnapshot {
        version: SNAPSHOT_VERSION,
        scratchpads: store.scratchpads(project)?,
        todos: store.todos(project)?,
        values: store.values(project)?,
    };
    let summary = format!(
        "exported {} scratchpads, {} todos, {} values to {}",
        snapshot.scratchpads.len(),
        snapshot.todos.len(),
        snapshot.values.len(),
        file.display(),
    );
    write_config(file, &snapshot)?;
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
    project: &Path,
    file: &Path,
) -> Result<Vec<Row>, CliError> {
    let raw = fs::read_to_string(file).map_err(|source| ConfigError::Read {
        path: file.to_path_buf(),
        source,
    })?;
    let snapshot: CoordinationSnapshot =
        serde_yaml_ng::from_str(&raw).map_err(ConfigError::from)?;
    if snapshot.version != SNAPSHOT_VERSION {
        return Err(ConfigError::UnsupportedCoordinationVersion(snapshot.version).into());
    }
    store.import(
        project,
        &snapshot.scratchpads,
        &snapshot.todos,
        &snapshot.values,
    )?;
    Ok(vec![Row::unlabeled(
        RowKind::Ok,
        format!(
            "imported {} scratchpads, {} todos, {} values from {}",
            snapshot.scratchpads.len(),
            snapshot.todos.len(),
            snapshot.values.len(),
            file.display(),
        ),
    )])
}
