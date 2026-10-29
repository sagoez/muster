use std::path::PathBuf;

/// Where the authoritative screen of a running process is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenSource {
    /// The supervisor's own rendering of the output it was sent.
    Mirror,
    /// A separate endpoint, listening at this socket, serves the screen.
    Served(PathBuf),
}
