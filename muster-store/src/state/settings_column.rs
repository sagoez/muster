//! The columns of the `settings` table, as its migrations name them.

use strum::{AsRefStr, EnumIter, IntoEnumIterator};

use crate::sql::{Column, ColumnSet};

/// A column of the one `settings` row, in the order its statements list
/// them. The names are the shipped schema's, which a migration never
/// renames.
#[derive(Clone, Copy, Debug, PartialEq, Eq, AsRefStr, EnumIter)]
pub(super) enum SettingsColumn {
    /// Whether to raise desktop notifications.
    #[strum(serialize = "desktop_notifications")]
    DesktopNotifications,
    /// The rows a process's terminal starts with.
    #[strum(serialize = "default_rows")]
    DefaultRows,
    /// The columns a process's terminal starts with.
    #[strum(serialize = "default_cols")]
    DefaultCols,
    /// The key that detaches `muster attach`.
    #[strum(serialize = "detach_key")]
    DetachKey,
    /// The size a log rotates at, in bytes.
    #[strum(serialize = "log_max_size_bytes")]
    LogMaxSizeBytes,
    /// Lines of scrollback a terminal keeps.
    #[strum(serialize = "scrollback_lines")]
    ScrollbackLines,
    /// The colors a terminal reports.
    #[strum(serialize = "color_scheme")]
    ColorScheme,
    /// Whether a new process autostarts.
    #[strum(serialize = "default_autostart")]
    DefaultAutostart,
    /// A new process's restart policy.
    #[strum(serialize = "default_restart")]
    DefaultRestart,
    /// A new process's stop signal.
    #[strum(serialize = "default_stop_signal")]
    DefaultStopSignal,
    /// A new process's stop grace, in milliseconds.
    #[strum(serialize = "default_stop_grace_ms")]
    DefaultStopGraceMs,
    /// Whether an agent may edit the stack.
    #[strum(serialize = "mcp_stack_edits")]
    McpStackEdits,
    /// How an idle stop is stored.
    #[strum(serialize = "idle_stop_kind")]
    IdleStopKind,
    /// An idle stop's delay, in milliseconds.
    #[strum(serialize = "idle_stop_ms")]
    IdleStopMs,
}

impl Column for SettingsColumn {}

impl SettingsColumn {
    /// Every column, as the settings row is read and written whole.
    pub(super) fn all() -> ColumnSet<Self> {
        ColumnSet::of(Self::iter())
    }
}
