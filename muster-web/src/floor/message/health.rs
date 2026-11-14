use serde::Serialize;

/// How the view's sources stand, which decides the page's banner: the
/// service it reads status from, then the state it reads projects from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Health {
    /// A service of this version answers.
    Running,
    /// No service runs.
    Absent,
    /// Another client is starting the service.
    Starting,
    /// A service of another version runs.
    OtherVersion,
    /// muster has no database yet.
    NoState,
    /// A newer muster wrote the database.
    NewerSchema,
    /// The database could not be read.
    Unreadable,
}
