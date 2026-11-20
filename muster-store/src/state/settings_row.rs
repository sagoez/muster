//! The one row of `settings`, as an update writes it.

use getset::Getters;
use rusqlite::types::Value;
use typed_builder::TypedBuilder;

use super::settings_column::SettingsColumn;
use crate::sql::RowSource;

/// Each value the settings row stores, already in its column's form.
#[derive(Clone, Debug, PartialEq, Getters, TypedBuilder)]
#[getset(get = "pub(super)")]
pub(super) struct SettingsRow {
    /// Whether to raise desktop notifications.
    desktop_notifications: bool,
    /// The rows a process's terminal starts with.
    default_rows: u16,
    /// The columns a process's terminal starts with.
    default_cols: u16,
    /// The key that detaches `muster attach`.
    detach_key: String,
    /// The size a log rotates at, in bytes.
    log_max_size_bytes: i64,
    /// Lines of scrollback a terminal keeps.
    scrollback_lines: i64,
    /// The colors a terminal reports.
    color_scheme: String,
    /// Whether a new process autostarts.
    default_autostart: bool,
    /// A new process's restart policy.
    default_restart: String,
    /// A new process's stop signal.
    default_stop_signal: String,
    /// A new process's stop grace, in milliseconds.
    default_stop_grace_ms: i64,
    /// Whether an agent may edit the stack.
    mcp_stack_edits: String,
    /// How the idle stop is stored.
    idle_stop_kind: String,
    /// The idle stop's delay, in milliseconds.
    idle_stop_ms: Option<i64>,
}

impl RowSource<SettingsColumn> for SettingsRow {
    fn value(&self, column: SettingsColumn) -> Value {
        match column {
            SettingsColumn::DesktopNotifications => self.desktop_notifications.into(),
            SettingsColumn::DefaultRows => self.default_rows.into(),
            SettingsColumn::DefaultCols => self.default_cols.into(),
            SettingsColumn::DetachKey => self.detach_key.clone().into(),
            SettingsColumn::LogMaxSizeBytes => self.log_max_size_bytes.into(),
            SettingsColumn::ScrollbackLines => self.scrollback_lines.into(),
            SettingsColumn::ColorScheme => self.color_scheme.clone().into(),
            SettingsColumn::DefaultAutostart => self.default_autostart.into(),
            SettingsColumn::DefaultRestart => self.default_restart.clone().into(),
            SettingsColumn::DefaultStopSignal => self.default_stop_signal.clone().into(),
            SettingsColumn::DefaultStopGraceMs => self.default_stop_grace_ms.into(),
            SettingsColumn::McpStackEdits => self.mcp_stack_edits.clone().into(),
            SettingsColumn::IdleStopKind => self.idle_stop_kind.clone().into(),
            SettingsColumn::IdleStopMs => self.idle_stop_ms.into(),
        }
    }
}
