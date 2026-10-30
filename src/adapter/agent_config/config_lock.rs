use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use super::{agent_config_error::AgentConfigError, config_file::io_error};

/// What Claude Code appends to its config's path for the folder that locks
/// it, as its `proper-lockfile` lock names it.
const LOCK_SUFFIX: &str = ".lock";
/// How long a lock folder may go untouched before it counts as left by a
/// writer that died: `proper-lockfile`'s default, which Claude Code keeps,
/// refreshing a live lock well within it.
const STALE_AFTER: Duration = Duration::from_secs(10);
/// The pauses between tries at a held lock, Claude Code's own: together
/// longer than a lock takes to go stale.
const RETRY_PAUSES: [Duration; 6] = [
    Duration::from_millis(200),
    Duration::from_millis(400),
    Duration::from_millis(800),
    Duration::from_millis(1600),
    Duration::from_millis(3200),
    Duration::from_millis(4000),
];
/// What a failed lock was doing.
const LOCK: &str = "lock";

/// The lock Claude Code takes around each re-read and write of its config,
/// held by muster across its own: a folder beside the file, made with
/// `mkdir` and removed when the guard drops. An RAII guard, so it has a
/// hand-written constructor.
#[derive(Debug)]
pub(super) struct ConfigLock {
    /// The lock folder.
    folder: PathBuf,
}

impl ConfigLock {
    /// Takes the lock of the config at `config`, the path as Claude Code
    /// names it, waiting while another writer holds it and taking over one
    /// gone stale. Without the config's folder there is nothing to lock,
    /// and none is taken.
    ///
    /// # Errors
    /// Returns [`AgentConfigError::Locked`] when the lock stays held,
    /// [`AgentConfigError::Interrupted`] when `cancelled` says so while
    /// waiting, or [`AgentConfigError::Io`] when the folder cannot be made.
    pub(super) fn take(
        config: &Path,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Self>, AgentConfigError> {
        if !config.parent().is_some_and(Path::is_dir) {
            return Ok(None);
        }
        let mut folder = OsString::from(config.as_os_str());
        folder.push(LOCK_SUFFIX);
        let folder = PathBuf::from(folder);
        let mut pauses = RETRY_PAUSES.into_iter();
        let mut stale_cleared = false;
        loop {
            match fs::create_dir(&folder) {
                Ok(()) => return Ok(Some(Self { folder })),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {},
                Err(source) => return Err(io_error(LOCK, config, source)),
            }
            if !stale_cleared && is_stale(&folder) {
                stale_cleared = true;
                let _ = fs::remove_dir(&folder);
                continue;
            }
            if cancelled() {
                return Err(AgentConfigError::Interrupted);
            }
            let Some(pause) = pauses.next() else {
                return Err(AgentConfigError::Locked(config.to_path_buf()));
            };
            stale_cleared = false;
            thread::sleep(pause);
        }
    }
}

impl Drop for ConfigLock {
    /// Lets Claude Code write its config again.
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.folder);
    }
}

/// Whether the lock folder at `folder` has gone untouched past
/// [`STALE_AFTER`].
fn is_stale(folder: &Path) -> bool {
    fs::metadata(folder)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age > STALE_AFTER)
}

#[cfg(test)]
mod tests {
    use std::{
        fs::File,
        time::{Instant, SystemTime},
    };

    use tempfile::TempDir;

    use super::*;
    use crate::adapter::agent_config::testing::uncancelled;

    /// How long a test holds a live lock before letting it go.
    const HELD_FOR: Duration = Duration::from_millis(300);
    /// How far in the past a stale lock was last touched.
    const LONG_AGO: Duration = Duration::from_secs(60);

    /// The config and lock folder paths in a scratch folder.
    fn paths(dir: &TempDir) -> (PathBuf, PathBuf) {
        (
            dir.path().join(".claude.json"),
            dir.path().join(".claude.json.lock"),
        )
    }

    /// The lock is a folder beside the config while held and gone once
    /// dropped; without the config's folder none is taken.
    #[test]
    fn the_lock_is_a_folder_beside_the_config() {
        let dir = TempDir::new().unwrap();
        let (config, folder) = paths(&dir);
        let lock = ConfigLock::take(&config, &uncancelled).unwrap();
        assert!(lock.is_some() && folder.is_dir());
        drop(lock);
        assert!(!folder.exists());
        let homeless = dir.path().join("missing/.claude.json");
        assert!(ConfigLock::take(&homeless, &uncancelled).unwrap().is_none());
    }

    /// A lock another writer holds is waited for, and taken once it is let
    /// go.
    #[test]
    fn a_held_lock_is_waited_for() {
        let dir = TempDir::new().unwrap();
        let (config, folder) = paths(&dir);
        fs::create_dir(&folder).unwrap();
        let held = folder.clone();
        let release = thread::spawn(move || {
            thread::sleep(HELD_FOR);
            fs::remove_dir(held).unwrap();
        });
        let started = Instant::now();
        let lock = ConfigLock::take(&config, &uncancelled).unwrap();
        assert!(started.elapsed() >= HELD_FOR);
        assert!(lock.is_some() && folder.is_dir());
        release.join().unwrap();
    }

    /// A lock untouched past the stale bound was left by a writer that
    /// died, so it is taken over; an interrupt while waiting gives up.
    #[test]
    fn a_stale_lock_is_taken_and_an_interrupt_stops_the_wait() {
        let dir = TempDir::new().unwrap();
        let (config, folder) = paths(&dir);
        fs::create_dir(&folder).unwrap();
        File::open(&folder)
            .unwrap()
            .set_modified(SystemTime::now() - LONG_AGO)
            .unwrap();
        let started = Instant::now();
        let lock = ConfigLock::take(&config, &uncancelled).unwrap();
        assert!(lock.is_some());
        assert!(started.elapsed() < RETRY_PAUSES[0], "taken without a wait");
        drop(lock);

        fs::create_dir(&folder).unwrap();
        assert!(matches!(
            ConfigLock::take(&config, &|| true),
            Err(AgentConfigError::Interrupted)
        ));
        assert!(folder.is_dir(), "another writer's lock is left alone");
    }
}
