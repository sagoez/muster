use serde::Serialize;

/// How the view's sources stand, which decides the page's banner: the
/// service it reads status from, then the state it reads projects from. A
/// reading that holds no data carries the health it found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Health {
    /// The view has not asked the service yet.
    Loading,
    /// A service of this version answers.
    Running,
    /// No service runs.
    Absent,
    /// Another client is starting the service.
    Starting,
    /// A service of another version runs.
    OtherVersion,
    /// A service holds its socket but has not answered several asks in a
    /// row.
    Unresponsive,
    /// muster has no database yet.
    NoState,
    /// An older muster wrote the database, which the next muster command
    /// migrates.
    OlderSchema,
    /// A newer muster wrote the database.
    NewerSchema,
    /// The database could not be read.
    Unreadable,
}
