use std::path::PathBuf;

/// Where the authoritative screen of a running process is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenSource {
    /// The last screen the handle repainted, which the supervisor keeps.
    Kept,
    /// A separate endpoint, listening at this socket, serves the screen.
    Served(PathBuf),
}
