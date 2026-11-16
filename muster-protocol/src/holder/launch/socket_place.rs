/// Which of its two folders a state directory's new holders bind their
/// sockets in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketPlace {
    /// The folder inside the state directory, where a socket fits.
    State,
    /// The fallback folder under `/tmp/muster-<uid>`, for a state directory
    /// too deep for a socket path to fit.
    Fallback,
}
