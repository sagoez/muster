use std::{io, path::PathBuf};

use muster_protocol::holder::HolderError;
use muster_service::ServiceError;
use thiserror::Error;

/// Why the state directory an earlier muster kept elsewhere was not moved to
/// where muster keeps it now. Every case but [`Self::Link`] leaves it where
/// it was, still in use.
#[derive(Debug, Error)]
pub enum StateMoveError {
    /// A service still runs from the old directory, and holds files there.
    #[error(
        "muster now keeps its state in {}, but a service still runs from {}, which stays in use \
         until it stops; run `muster service stop` (every process keeps running) and the next \
         muster command moves it",
        to.display(),
        from.display()
    )]
    ServiceRunning {
        /// The old directory.
        from: PathBuf,
        /// Where it moves to.
        to: PathBuf,
    },
    /// Whether a service runs from the old directory could not be told.
    #[error("could not tell whether a service runs from {}, so it is not moved yet", from.display())]
    Lock {
        /// The old directory.
        from: PathBuf,
        /// What failed.
        #[source]
        source: ServiceError,
    },
    /// The holder records could not be made to name the new directory.
    #[error(
        "could not point the process records in {} at {}, so it is not moved yet",
        from.display(),
        to.display()
    )]
    Records {
        /// The old directory.
        from: PathBuf,
        /// Where it moves to.
        to: PathBuf,
        /// What failed.
        #[source]
        source: HolderError,
    },
    /// The directory, or the folder it moves into, could not be moved or made.
    #[error("could not move {} to {}, so it stays in use", from.display(), to.display())]
    Move {
        /// The old directory.
        from: PathBuf,
        /// Where it moves to.
        to: PathBuf,
        /// What failed.
        #[source]
        source: io::Error,
    },
    /// The directory moved, but no link to it could be left in its old
    /// place, through which processes still running reach their files.
    #[error(
        "moved {} to {}, but could not leave a link in its old place; processes that were \
         already running cannot rotate their logs until they restart",
        from.display(),
        to.display()
    )]
    Link {
        /// The old directory.
        from: PathBuf,
        /// Where it moved to.
        to: PathBuf,
        /// What failed.
        #[source]
        source: io::Error,
    },
}
