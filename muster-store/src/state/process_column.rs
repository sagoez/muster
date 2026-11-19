//! The columns of the `processes` table, as its migrations name them.

use strum::{AsRefStr, EnumIter, EnumProperty, IntoEnumIterator};

use crate::sql::{Column, ColumnSet};

/// The property marking the row id, which only a read names.
const ROW_ID: &str = "row_id";
/// The property marking a column that finds a process's row.
const ROW_KEY: &str = "row_key";

/// A column of `processes`, in the order the table's statements list them.
/// The names are the shipped schema's, which a migration never renames.
#[derive(Clone, Copy, Debug, PartialEq, Eq, AsRefStr, EnumIter, EnumProperty)]
pub(super) enum ProcessColumn {
    /// The row id, which orders a project's processes.
    #[strum(serialize = "id", props(row_id = true))]
    Id,
    /// The project the process belongs to.
    #[strum(serialize = "project_id", props(row_key = true))]
    ProjectId,
    /// The name's key, unique in its project.
    #[strum(serialize = "name_key", props(row_key = true))]
    NameKey,
    /// The name as written.
    #[strum(serialize = "name")]
    Name,
    /// The command line.
    #[strum(serialize = "command")]
    Command,
    /// The folder it runs in, when not the project's.
    #[strum(serialize = "working_dir")]
    WorkingDir,
    /// What it is for.
    #[strum(serialize = "description")]
    Description,
    /// Whether `muster up` starts it.
    #[strum(serialize = "autostart")]
    Autostart,
    /// Its restart policy.
    #[strum(serialize = "restart")]
    Restart,
    /// Its graceful stop signal.
    #[strum(serialize = "stop_signal")]
    StopSignal,
    /// Its grace before a force-kill, in milliseconds.
    #[strum(serialize = "stop_grace_ms")]
    StopGraceMs,
    /// Its readiness probe's kind.
    #[strum(serialize = "ready_kind")]
    ReadyKind,
    /// What its readiness probe checks.
    #[strum(serialize = "ready_target")]
    ReadyTarget,
    /// How long its readiness probe waits, in milliseconds.
    #[strum(serialize = "ready_timeout_ms")]
    ReadyTimeoutMs,
    /// Its icon.
    #[strum(serialize = "icon")]
    Icon,
}

impl Column for ProcessColumn {}

impl ProcessColumn {
    /// The columns that find a process's row: its project and name key.
    pub(super) fn row_key() -> ColumnSet<Self> {
        ColumnSet::of(Self::iter().filter(|column| column.flag(ROW_KEY)))
    }

    /// The columns of a process's definition, which an update sets.
    pub(super) fn definition() -> ColumnSet<Self> {
        ColumnSet::of(Self::iter().filter(|column| !column.flag(ROW_ID) && !column.flag(ROW_KEY)))
    }

    /// The columns a read takes: the row id and the definition.
    pub(super) fn read() -> ColumnSet<Self> {
        ColumnSet::of(Self::iter().filter(|column| !column.flag(ROW_KEY)))
    }

    /// The columns an insert writes: the row key and the definition.
    pub(super) fn written() -> ColumnSet<Self> {
        ColumnSet::of(Self::iter().filter(|column| !column.flag(ROW_ID)))
    }

    /// Whether this column carries `property`.
    fn flag(self, property: &str) -> bool {
        self.get_bool(property).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sql::{ColumnForm, SqlList};

    /// Each statement lists the columns it did before they were typed.
    #[test]
    fn each_statement_lists_its_columns() {
        assert_eq!(
            ProcessColumn::read().written(ColumnForm::Name, SqlList::Commas),
            "id, name, command, working_dir, description, autostart, restart, stop_signal, \
             stop_grace_ms, ready_kind, ready_target, ready_timeout_ms, icon"
        );
        assert_eq!(
            ProcessColumn::written().written(ColumnForm::Name, SqlList::Commas),
            "project_id, name_key, name, command, working_dir, description, autostart, \
             restart, stop_signal, stop_grace_ms, ready_kind, ready_target, ready_timeout_ms, \
             icon"
        );
        assert_eq!(
            ProcessColumn::row_key().written(ColumnForm::Equals, SqlList::And),
            "project_id = :project_id AND name_key = :name_key"
        );
        assert!(
            ProcessColumn::definition()
                .written(ColumnForm::Equals, SqlList::Commas)
                .starts_with("name = :name, command = :command, ")
        );
    }
}
