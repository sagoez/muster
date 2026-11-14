use std::io;

use muster_os::no_state_dir::NoStateDir;
use opener::OpenError;
use thiserror::Error;

/// A failure starting or running `muster view`.
#[derive(Debug, Error)]
pub enum WebError {
    /// No state directory to find the processes' logs under.
    #[error(transparent)]
    NoStateDir(#[from] NoStateDir),
    /// The loopback port could not be listened on.
    #[error("could not listen on 127.0.0.1: {0}")]
    Bind(#[source] io::Error),
    /// No random token could be made.
    #[error("could not make the view's token: {0}")]
    Token(#[source] io::Error),
    /// The server's thread could not start.
    #[error("could not start the view's server: {0}")]
    Start(#[source] io::Error),
    /// The server's thread stopped.
    #[error("the view's server stopped")]
    Stopped,
    /// The default browser could not be opened.
    #[error("{0}")]
    Browser(#[source] OpenError),
}
