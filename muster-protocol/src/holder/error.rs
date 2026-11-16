use std::{
    io,
    path::{Path, PathBuf},
};

use muster_domain::{pty::PtyError, value::OutputCursor};
use muster_os::private_dir::{FALLBACK_SOCKET_ROOT, PRIVATE_DIR_MODE, is_current_user};
use serde_json::Error as JsonError;
use thiserror::Error;

use super::socket::frame_tag::FrameTag;

/// How a refused socket folder's owner is named when it could not be read.
const UNKNOWN_OWNER: &str = "owner unknown";
/// What comes before the uid of a refused socket folder's owner.
const OWNED_BY: &str = "owned by uid ";

/// Who owns a refused socket folder, as its error says it.
fn owner_text(owner: Option<u32>) -> String {
    owner.map_or_else(
        || UNKNOWN_OWNER.to_string(),
        |owner| format!("{OWNED_BY}{owner}"),
    )
}

/// What clears the refusal of the socket folder `dir` that `owner` owns:
/// one of the user's own only needs its mode fixed, while one another user
/// made, at a fixed path in `/tmp`, must be removed by them or by a reboot.
fn remedy(dir: &Path, owner: Option<u32>) -> String {
    if owner.is_some_and(is_current_user) {
        return format!(
            "make it private with `chmod {PRIVATE_DIR_MODE:o} {}`",
            dir.display()
        );
    }
    format!(
        "have its owner or an administrator remove it, or reboot to clear {FALLBACK_SOCKET_ROOT}"
    )
}

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
    /// The socket's directory, or the folder holding it, is not a real
    /// directory this user owns that no other user can enter. Its fixed path
    /// cannot move, so the message says who owns it and what clears it.
    #[error(
        "the holder socket folder {} ({}) is not a directory only you can enter, so muster will \
         not use it; {}",
        dir.display(),
        owner_text(*owner),
        remedy(dir, *owner)
    )]
    InsecureSocketDir {
        /// The folder refused.
        dir: PathBuf,
        /// The uid that owns it, when it could be read.
        owner: Option<u32>,
    },
    /// A holder socket path names no folder to bind it in.
    #[error("the holder socket path {0} names no folder")]
    SocketWithoutFolder(PathBuf),
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
            | Self::InsecureSocketDir { .. }
            | Self::SocketWithoutFolder(_)
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
            other @ (HolderError::Json(_)
            | HolderError::UnexpectedFrame(_)
            | HolderError::MalformedPayload(_)
            | HolderError::MissingProject
            | HolderError::MissingChildPid
            | HolderError::UnrecordablePid(_)
            | HolderError::MissingChildInput
            | HolderError::InsecureSocketDir { .. }
            | HolderError::SocketWithoutFolder(_)
            | HolderError::NotReady(_)
            | HolderError::NoSnapshot(_)
            | HolderError::FailedToStart { .. }
            | HolderError::CursorAhead { .. }) => Self::System(other.to_string()),
        }
    }
}
