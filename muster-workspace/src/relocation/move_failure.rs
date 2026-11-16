//! Why a service's own move of the old state directory failed, kept in that
//! directory while the service runs from it, so every command says why the
//! state has not moved instead of taking that service for an older one.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

use muster_domain::constants::APP_VERSION;

/// File in the old state directory holding the version of the service that
/// failed to move it, then why.
const MOVE_FAILURE_FILE: &str = "state-move.failed";
/// Mode of that file: its owner's alone.
const MOVE_FAILURE_MODE: u32 = 0o600;
/// Between the version and the reason in that file.
const VERSION_END: char = '\n';

/// Records in `dir` that a service of this version could not move it, for
/// `reason`. A record that cannot be written is left out: the service log
/// still has the reason.
pub fn record_move_failure(dir: &Path, reason: &str) {
    let _ = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(MOVE_FAILURE_MODE)
        .open(dir.join(MOVE_FAILURE_FILE))
        .and_then(|mut file| write!(file, "{APP_VERSION}{VERSION_END}{reason}"));
}

/// Why a service of this version could not move `dir`, as it recorded
/// there; none when no such service recorded one, as an older one never
/// does.
pub fn recorded_move_failure(dir: &Path) -> Option<String> {
    let recorded = fs::read_to_string(dir.join(MOVE_FAILURE_FILE)).ok()?;
    let (version, reason) = recorded.split_once(VERSION_END)?;
    (version == APP_VERSION).then(|| reason.to_string())
}

/// Removes the failure recorded in `dir`, as once it has moved.
pub fn clear_move_failure(dir: &Path) {
    let _ = fs::remove_file(dir.join(MOVE_FAILURE_FILE));
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    /// What a test records.
    const REASON: &str = "the disk is full";
    /// A version no build of this crate runs.
    const OTHER_VERSION: &str = "0.0.1";

    /// A recorded failure reads back until it is cleared; one another
    /// version wrote is not this version's.
    #[test]
    fn a_failure_reads_back_only_for_this_version() {
        let scratch = TempDir::new().unwrap();

        record_move_failure(scratch.path(), REASON);
        assert_eq!(
            recorded_move_failure(scratch.path()).as_deref(),
            Some(REASON)
        );
        clear_move_failure(scratch.path());
        assert_eq!(recorded_move_failure(scratch.path()), None);

        fs::write(
            scratch.path().join(MOVE_FAILURE_FILE),
            format!("{OTHER_VERSION}{VERSION_END}{REASON}"),
        )
        .unwrap();
        assert_eq!(recorded_move_failure(scratch.path()), None);
    }
}
