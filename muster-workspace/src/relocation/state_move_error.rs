use std::{io, path::PathBuf};

use thiserror::Error;

/// Why the state directory an earlier muster kept elsewhere was not moved to
/// where muster keeps it now, said so the person knows where their state
/// is. Every case but [`Self::Unlinked`] leaves the old directory whole and
/// in use, or, for [`Self::LeftBehind`], untouched.
#[derive(Debug, Error)]
pub enum StateMoveError {
    /// A service, an older muster's as a rule, still runs from the old
    /// directory and holds its lock.
    #[error(
        "muster now keeps its state in {}, but a muster service still runs from {}, which stays \
         in use until it stops; run `muster service stop` (every process keeps running) and the \
         next muster command moves it",
        to.display(),
        from.display()
    )]
    ServiceRunning {
        /// The old directory.
        from: PathBuf,
        /// Where it moves to.
        to: PathBuf,
    },
    /// The state directory already exists, and the old one still holds a
    /// database that is not merged into it.
    #[error(
        "muster keeps its state in {}; {}, where an earlier muster kept it, still holds a \
         database that was not merged; copy what you need from it, or remove it",
        to.display(),
        from.display()
    )]
    LeftBehind {
        /// The old directory.
        from: PathBuf,
        /// The state directory in use.
        to: PathBuf,
    },
    /// Whether a service runs from the old directory could not be told.
    #[error(
        "could not tell whether a service runs from {}, so it is not moved yet: {cause}",
        from.display()
    )]
    Lock {
        /// The old directory.
        from: PathBuf,
        /// What failed.
        cause: io::Error,
    },
    /// The directory, or the folder it moves into, could not be moved or
    /// made.
    #[error(
        "could not move {} to {}, so it stays in use: {cause}",
        from.display(),
        to.display()
    )]
    Move {
        /// The old directory.
        from: PathBuf,
        /// Where it moves to.
        to: PathBuf,
        /// What failed.
        cause: io::Error,
    },
    /// The state directory is on another disk, where a directory cannot be
    /// renamed to.
    #[error(
        "could not move {} to {}, which is on another disk, so it stays in use; set \
         XDG_STATE_HOME to a folder of your own on the same disk",
        from.display(),
        to.display()
    )]
    OtherDisk {
        /// The old directory.
        from: PathBuf,
        /// Where it moves to.
        to: PathBuf,
    },
    /// No link to the new place could be left in the old one, through which
    /// running processes reach their files, so nothing moved, or it was
    /// moved back.
    #[error(
        "could not leave a link at {} leading to {}, so it was not moved and stays in use: \
         {cause}",
        from.display(),
        to.display()
    )]
    Link {
        /// The old directory.
        from: PathBuf,
        /// Where it moves to.
        to: PathBuf,
        /// What failed.
        cause: io::Error,
    },
    /// The directory moved, but neither the link to it nor the move back
    /// could be made.
    #[error(
        "moved {} to {}, but could neither leave a link in its old place nor move it back: \
         {cause}; the next muster command makes the link",
        from.display(),
        to.display()
    )]
    Unlinked {
        /// The old directory.
        from: PathBuf,
        /// Where it moved to.
        to: PathBuf,
        /// What failed.
        cause: io::Error,
    },
}
