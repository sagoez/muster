//! A process's row of `processes`, as an insert or update writes it.

use getset::Getters;
use rusqlite::types::Value;
use typed_builder::TypedBuilder;

use super::process_column::ProcessColumn;
use crate::sql::RowSource;

/// Each value a process's row stores, already in its column's form.
#[derive(Clone, Debug, PartialEq, Getters, TypedBuilder)]
#[getset(get = "pub(super)")]
pub(super) struct ProcessRow {
    /// The project's row id.
    project_id: i64,
    /// The name's key.
    name_key: String,
    /// The name as written.
    name: String,
    /// The command line.
    command: String,
    /// The working directory, when not the project's.
    working_dir: Option<String>,
    /// The description.
    description: Option<String>,
    /// Whether `muster up` starts it.
    autostart: bool,
    /// The restart policy.
    restart: String,
    /// The stop signal.
    stop_signal: String,
    /// The stop grace, in milliseconds.
    stop_grace_ms: i64,
    /// The readiness probe's kind.
    ready_kind: Option<String>,
    /// What the readiness probe checks.
    ready_target: Option<String>,
    /// The readiness timeout, in milliseconds.
    ready_timeout_ms: Option<i64>,
    /// The icon.
    icon: Option<String>,
}

impl RowSource<ProcessColumn> for ProcessRow {
    fn value(&self, column: ProcessColumn) -> Value {
        match column {
            // SQLite assigns the row id; no statement writes it.
            ProcessColumn::Id => Value::Null,
            ProcessColumn::ProjectId => self.project_id.into(),
            ProcessColumn::NameKey => self.name_key.clone().into(),
            ProcessColumn::Name => self.name.clone().into(),
            ProcessColumn::Command => self.command.clone().into(),
            ProcessColumn::WorkingDir => self.working_dir.clone().into(),
            ProcessColumn::Description => self.description.clone().into(),
            ProcessColumn::Autostart => self.autostart.into(),
            ProcessColumn::Restart => self.restart.clone().into(),
            ProcessColumn::StopSignal => self.stop_signal.clone().into(),
            ProcessColumn::StopGraceMs => self.stop_grace_ms.into(),
            ProcessColumn::ReadyKind => self.ready_kind.clone().into(),
            ProcessColumn::ReadyTarget => self.ready_target.clone().into(),
            ProcessColumn::ReadyTimeoutMs => self.ready_timeout_ms.into(),
            ProcessColumn::Icon => self.icon.clone().into(),
        }
    }
}
