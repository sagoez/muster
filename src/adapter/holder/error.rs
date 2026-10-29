use std::{io, path::PathBuf};

use serde_json::Error as JsonError;
use thiserror::Error;

use super::frame::FrameTag;
use crate::domain::{pty::PtyError, value::OutputCursor};

/// Errors from running a holder or speaking its protocol.
#[derive(Debug, Error)]
pub enum HolderError {
    /// A socket, log or record file operation failed.
    #[error("holder i/o error: {0}")]
    Io(#[from] io::Error),
    /// Spawning or driving the child under its PTY failed.
    #[error(transparent)]
    Pty(#[from] PtyError),
    /// A message body could not be encoded or decoded.
    #[error("holder message is not valid json: {0}")]
    Json(#[from] JsonError),
    /// A frame arrived that this side of the protocol never receives.
    #[error("unexpected holder frame: {0:?}")]
    UnexpectedFrame(FrameTag),
    /// A frame's payload does not have the shape its tag requires.
    #[error("malformed holder frame payload: {0:?}")]
    MalformedPayload(FrameTag),
    /// The spawn request names no project, which the holder record requires.
    #[error("the spawn request names no project")]
    MissingProject,
    /// The platform reported no pid for the spawned child.
    #[error("the spawned child has no process id")]
    MissingChildPid,
    /// The holder runs as a pid no record can name.
    #[error("the holder runs as pid {0}, which no holder record can name")]
    UnrecordablePid(u32),
    /// The spawned child exposes no input writer the holder can drive.
    #[error("the spawned child has no input writer")]
    MissingChildInput,
    /// The socket's directory is reachable by other users, or there is none.
    #[error("holder socket directory {0} is not private to its owner")]
    InsecureSocketDir(PathBuf),
    /// A launched holder did not accept a connection within the ready timeout.
    #[error("holder at {0} did not become ready in time")]
    NotReady(PathBuf),
    /// The holder answered a supervisor's hello with something other than its
    /// screen, or closed the connection first.
    #[error("holder at {0} did not send its screen")]
    NoSnapshot(PathBuf),
    /// A launched holder reported the error it failed with before serving.
    #[error("holder at {socket} failed to start: {reason}")]
    FailedToStart {
        /// The socket the holder was to serve.
        socket: PathBuf,
        /// The error the holder reported.
        reason: String,
    },
    /// A log read was given a cursor past the end of the process's output.
    #[error("cursor {cursor} is past the end of the output, {end}")]
    CursorAhead {
        /// The cursor given.
        cursor: OutputCursor,
        /// The end of the output.
        end: OutputCursor,
    },
}

impl HolderError {
    /// Whether the peer sent bytes that are not a valid message, as opposed to
    /// the connection closing or failing underneath.
    pub fn is_undecodable(&self) -> bool {
        match self {
            Self::Io(error) => error.kind() == io::ErrorKind::InvalidData,
            Self::Json(_) | Self::UnexpectedFrame(_) | Self::MalformedPayload(_) => true,
            Self::Pty(_)
            | Self::MissingProject
            | Self::MissingChildPid
            | Self::UnrecordablePid(_)
            | Self::MissingChildInput
            | Self::InsecureSocketDir(_)
            | Self::NotReady(_)
            | Self::NoSnapshot(_)
            | Self::FailedToStart { .. }
            | Self::CursorAhead { .. } => false,
        }
    }
}

impl From<HolderError> for PtyError {
    /// Carries a holder failure across the runner port, keeping I/O and PTY
    /// errors as they are.
    fn from(error: HolderError) -> Self {
        match error {
            HolderError::Io(error) => Self::Io(error),
            HolderError::Pty(error) => error,
            other => Self::System(other.to_string()),
        }
    }
}
