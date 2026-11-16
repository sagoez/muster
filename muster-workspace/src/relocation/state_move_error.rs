use std::{io, path::PathBuf};

use thiserror::Error;

/// Why the state directory an earlier muster kept elsewhere was not moved to
/// where muster keeps it now, said so the person knows where their state
/// is. Every case but [`Self::Unlinked`] leaves the old directory whole and
/// in use, or, for [`Self::LeftBehind`], untouched.
#[derive(Debug, Error)]
pub enum StateMoveError {
    /// A service of an earlier version still runs from the old directory
    /// and holds its lock.
    #[error(
        "an older muster service still runs from {}, which stays in use; the next muster command \
         that reaches the service hands over to this version, which moves it to {}, or run \
         `muster service stop` (every process keeps running)",
        from.display(),
        to.display()
    )]
    ServiceRunning {
        /// The old directory.
        from: PathBuf,
        /// Where it moves to.
        to: PathBuf,
    },
    /// A service of this version runs from the old directory because its
    /// own move failed, for the reason it recorded there.
    #[error(
        "the muster service could not move its state, so {} stays in use: {reason}; once that is \
         fixed, `muster service stop` lets the next command move it",
        from.display()
    )]
    MoveFailed {
        /// The old directory.
        from: PathBuf,
        /// Why the service's move failed.
        reason: String,
    },
    /// The state directory already exists, and the folder the old place
    /// leads to still holds a database that is not merged into it.
    #[error(
        "muster keeps its state in {}; {}, where an earlier muster kept it, still holds a \
         database that was not merged; copy what you need from it",
        to.display(),
        found.display()
    )]
    LeftBehind {
        /// The folder the old place leads to, where that database is.
        found: PathBuf,
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
    /// The new place is where this user may not keep state: under another
    /// user's folder, or in one others may write.
    #[error("{} is not moved, so it stays in use: {cause}", from.display())]
    Target {
        /// The old directory.
        from: PathBuf,
        /// Why the new place was refused.
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
        "could not move {} to {}, which is on another disk, so it stays in use; set {variable} to \
         a folder of your own on the same disk",
        from.display(),
        to.display()
    )]
    OtherDisk {
        /// The old directory.
        from: PathBuf,
        /// Where it moves to.
        to: PathBuf,
        /// The variable that chooses where the state goes.
        variable: &'static str,
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

impl StateMoveError {
    /// Whether this is a move that was tried and failed, which a service
    /// records for every command to report, rather than one held up by a
    /// service, already reported, or never due.
    pub fn is_failed_move(&self) -> bool {
        !matches!(
            self,
            Self::ServiceRunning { .. } | Self::MoveFailed { .. } | Self::LeftBehind { .. }
        )
    }
}
