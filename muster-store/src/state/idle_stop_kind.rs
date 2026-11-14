use strum::{Display, EnumString};

/// How an idle stop is stored: the `idle_stop_kind` column of the settings
/// row and a project's row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumString)]
#[strum(serialize_all = "kebab-case")]
pub(super) enum IdleStopKind {
    /// Stops once the project has been empty for `idle_stop_ms`.
    After,
    /// Never stops for being empty.
    Never,
}
