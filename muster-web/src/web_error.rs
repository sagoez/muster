use std::io;

use muster_os::no_state_dir::NoStateDir;
use thiserror::Error;

/// A failure starting or running `muster view`. Each message carries its
/// cause, so none is a `source` as well and the cause prints once.
#[derive(Debug, Error)]
pub enum WebError {
    /// No state directory to find the processes' logs under.
    #[error(transparent)]
    NoStateDir(#[from] NoStateDir),
    /// The loopback port could not be listened on.
    #[error("could not listen on 127.0.0.1: {0}")]
    Bind(io::Error),
    /// No random token could be made.
    #[error("could not make the view's token: {0}")]
    Token(io::Error),
    /// The server's thread could not start.
    #[error("could not start the view's server: {0}")]
    Start(io::Error),
    /// Ctrl-C and the other exit signals could not be caught.
    #[error("could not catch the view's exit signals: {0}")]
    Signals(io::Error),
    /// No one-time code to open the browser with could be made.
    #[error("could not make the view's one-time code: {0}")]
    Code(io::Error),
    /// The default browser could not be opened; the CLI says so in its own
    /// words, so the message is the cause alone.
    #[error("{0}")]
    Browser(io::Error),
    /// A message for the page could not be written as JSON.
    #[error("could not write a message for the page: {0}")]
    Frame(serde_json::Error),
    /// The thread that owns the floor ended while the view still served.
    #[error("the view stopped working: its floor's thread ended")]
    HubEnded,
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    /// A failure's cause prints once: in its message, never again as a
    /// source a reporter would print after it.
    #[test]
    fn a_cause_prints_once() {
        let cause = || io::Error::other("no such browser");

        for error in [WebError::Browser(cause()), WebError::Bind(cause())] {
            assert_eq!(error.to_string().matches("no such browser").count(), 1);
            assert!(error.source().is_none(), "{error}");
        }
    }
}
