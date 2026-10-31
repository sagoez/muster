use std::{
    ffi::OsString,
    fs::{self, File},
    io,
    path::{Path, PathBuf},
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, bounded};

use super::{agent_config_error::AgentConfigError, config_file::io_error, lock_timing::LockTiming};

/// What Claude Code appends to its config's path for the folder that locks
/// it, as its `proper-lockfile` lock names it.
const LOCK_SUFFIX: &str = ".lock";
/// What a failed lock was doing.
const LOCK: &str = "lock";
/// The name of the thread keeping a held lock fresh.
const REFRESH_THREAD: &str = "config-lock-refresh";

/// The lock Claude Code takes around each re-read and write of its config,
/// held by muster across its own: a folder beside the file, made with
/// `mkdir`, touched while held so it never looks stale, and removed when the
/// guard drops only if it is still muster's. An RAII guard, so it has a
/// hand-written constructor.
#[derive(Debug)]
pub(super) struct ConfigLock {
    /// The lock folder.
    folder: PathBuf,
    /// Dropped to stop the refresh.
    stop: Option<Sender<()>>,
    /// The refresh, giving back the last time it stamped on the folder, or
    /// none once another writer took the lock over.
    refresh: Option<JoinHandle<Option<SystemTime>>>,
}

impl ConfigLock {
    /// Takes the lock of the config at `config`, the path as Claude Code
    /// names it, waiting while another writer holds it and taking over one
    /// gone stale, as `timing` says. Without the config's folder there is
    /// nothing to lock, and none is taken.
    ///
    /// # Errors
    /// Returns [`AgentConfigError::Locked`] when the lock stays held,
    /// [`AgentConfigError::Interrupted`] when `cancelled` says so while
    /// waiting, or [`AgentConfigError::Io`] when the folder cannot be made
    /// or kept fresh.
    pub(super) fn take(
        config: &Path,
        timing: &LockTiming,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Self>, AgentConfigError> {
        if !config.parent().is_some_and(Path::is_dir) {
            return Ok(None);
        }
        let mut folder = OsString::from(config.as_os_str());
        folder.push(LOCK_SUFFIX);
        let folder = PathBuf::from(folder);
        let mut pauses = timing.retry_pauses().iter();
        let mut stale_cleared = false;
        loop {
            match fs::create_dir(&folder) {
                Ok(()) => return Self::hold(folder, timing.refresh_every(), config).map(Some),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {},
                Err(source) => return Err(io_error(LOCK, config, source)),
            }
            if !stale_cleared && is_stale(&folder, timing.stale_after()) {
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
            thread::sleep(*pause);
        }
    }

    /// The guard of `folder`, just made: stamped as muster's and touched
    /// every `refresh_every` from a thread of its own.
    ///
    /// # Errors
    /// Returns [`AgentConfigError::Io`] when the folder cannot be stamped or
    /// the thread started; the folder is removed again.
    fn hold(
        folder: PathBuf,
        refresh_every: Duration,
        config: &Path,
    ) -> Result<Self, AgentConfigError> {
        let start = || -> io::Result<_> {
            let stamp = stamp(&folder)?;
            let (stop, stopped) = bounded(0);
            let watched = folder.clone();
            let refresh = thread::Builder::new()
                .name(REFRESH_THREAD.to_owned())
                .spawn(move || refresh(&watched, stamp, refresh_every, &stopped))?;
            Ok((stop, refresh))
        };
        match start() {
            Ok((stop, refresh)) => Ok(Self {
                folder,
                stop: Some(stop),
                refresh: Some(refresh),
            }),
            Err(source) => {
                let _ = fs::remove_dir(&folder);
                Err(io_error(LOCK, config, source))
            },
        }
    }
}

impl Drop for ConfigLock {
    /// Stops the refresh and lets Claude Code write its config again,
    /// unless another writer took the lock over meanwhile: its folder is
    /// then left alone.
    fn drop(&mut self) {
        drop(self.stop.take());
        let last = self
            .refresh
            .take()
            .and_then(|refresh| refresh.join().ok())
            .flatten();
        if last.is_some() && modified(&self.folder) == last {
            let _ = fs::remove_dir(&self.folder);
        }
    }
}

/// Touches `folder` every `every` while it still bears `stamp`, the last
/// time muster set on it, until `stopped` says to stop.
///
/// Returns the last stamp, or none once the folder no longer bears it.
fn refresh(
    folder: &Path,
    mut stamp: SystemTime,
    every: Duration,
    stopped: &Receiver<()>,
) -> Option<SystemTime> {
    loop {
        match stopped.recv_timeout(every) {
            Err(RecvTimeoutError::Timeout) => {
                if modified(folder) != Some(stamp) {
                    return None;
                }
                stamp = self::stamp(folder).ok()?;
            },
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return Some(stamp),
        }
    }
}

/// Sets `folder`'s modified time to now, in whole seconds so every file
/// system keeps it exactly as set, and returns it.
///
/// # Errors
/// Returns the failure to set it.
fn stamp(folder: &Path) -> io::Result<SystemTime> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let now = UNIX_EPOCH + Duration::from_secs(seconds);
    File::open(folder)?.set_modified(now)?;
    Ok(now)
}

/// When `folder` was last modified, if it is there.
fn modified(folder: &Path) -> Option<SystemTime> {
    fs::metadata(folder).and_then(|meta| meta.modified()).ok()
}

/// Whether the lock folder at `folder` has gone untouched past
/// `stale_after`.
fn is_stale(folder: &Path, stale_after: Duration) -> bool {
    modified(folder)
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age > stale_after)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use tempfile::TempDir;

    use super::*;
    use crate::adapter::agent_config::testing::uncancelled;

    /// How long a test holds a live lock before letting it go.
    const HELD_FOR: Duration = Duration::from_millis(300);
    /// How far in the past a stale lock was last touched.
    const LONG_AGO: Duration = Duration::from_secs(60);
    /// The stale bound of a test's quick lock.
    const QUICK_STALE: Duration = Duration::from_secs(2);
    /// How often a test's quick lock is touched.
    const QUICK_REFRESH: Duration = Duration::from_millis(200);
    /// A pause between tries at a test's quick lock.
    const QUICK_PAUSE: Duration = Duration::from_millis(250);
    /// How many tries a test's quick lock waits through: together past
    /// its stale bound.
    const QUICK_TRIES: usize = 10;
    /// How much longer than its stale bound a test holds a quick lock.
    const PAST_STALE: Duration = Duration::from_millis(500);

    /// The config and lock folder paths in a scratch folder.
    fn paths(dir: &TempDir) -> (PathBuf, PathBuf) {
        (
            dir.path().join(".claude.json"),
            dir.path().join(".claude.json.lock"),
        )
    }

    /// Claude Code's timing.
    fn claude() -> LockTiming {
        LockTiming::claude_code()
    }

    /// A timing that goes stale, and is waited past, within seconds.
    fn quick() -> LockTiming {
        LockTiming::builder()
            .stale_after(QUICK_STALE)
            .refresh_every(QUICK_REFRESH)
            .retry_pauses(vec![QUICK_PAUSE; QUICK_TRIES])
            .build()
    }

    /// Sets `folder`'s modified time to `at`, as another writer would.
    fn touch(folder: &Path, at: SystemTime) {
        File::open(folder).unwrap().set_modified(at).unwrap();
    }

    /// The lock is a folder beside the config while held and gone once
    /// dropped; without the config's folder none is taken.
    #[test]
    fn the_lock_is_a_folder_beside_the_config() {
        let dir = TempDir::new().unwrap();
        let (config, folder) = paths(&dir);
        let lock = ConfigLock::take(&config, &claude(), &uncancelled).unwrap();
        assert!(lock.is_some() && folder.is_dir());
        drop(lock);
        assert!(!folder.exists());
        let homeless = dir.path().join("missing/.claude.json");
        assert!(
            ConfigLock::take(&homeless, &claude(), &uncancelled)
                .unwrap()
                .is_none()
        );
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
        let lock = ConfigLock::take(&config, &claude(), &uncancelled).unwrap();
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
        touch(&folder, SystemTime::now() - LONG_AGO);
        let started = Instant::now();
        let lock = ConfigLock::take(&config, &claude(), &uncancelled).unwrap();
        assert!(lock.is_some());
        assert!(
            started.elapsed() < claude().retry_pauses()[0],
            "taken without a wait"
        );
        drop(lock);

        fs::create_dir(&folder).unwrap();
        assert!(matches!(
            ConfigLock::take(&config, &claude(), &|| true),
            Err(AgentConfigError::Interrupted)
        ));
        assert!(folder.is_dir(), "another writer's lock is left alone");
    }

    /// A lock muster holds past the stale bound stays fresh, so another
    /// writer waits it out rather than taking it over.
    #[test]
    fn a_hold_past_the_stale_bound_is_not_taken_over() {
        let dir = TempDir::new().unwrap();
        let (config, folder) = paths(&dir);
        let held = ConfigLock::take(&config, &quick(), &uncancelled).unwrap();
        thread::sleep(QUICK_STALE + PAST_STALE);
        assert!(!is_stale(&folder, QUICK_STALE), "kept fresh");
        assert!(matches!(
            ConfigLock::take(&config, &quick(), &uncancelled),
            Err(AgentConfigError::Locked(_))
        ));
        assert!(folder.is_dir(), "still held");
        drop(held);
        assert!(!folder.exists());
    }

    /// A lock another writer took over, by making the folder again or
    /// touching it, is neither touched again nor removed by muster.
    #[test]
    fn a_lock_taken_over_is_left_to_its_new_holder() {
        let dir = TempDir::new().unwrap();
        let (config, folder) = paths(&dir);

        let remade = ConfigLock::take(&config, &quick(), &uncancelled).unwrap();
        fs::remove_dir(&folder).unwrap();
        fs::create_dir(&folder).unwrap();
        let theirs = modified(&folder);
        thread::sleep(QUICK_REFRESH * 3);
        assert_eq!(modified(&folder), theirs, "no longer touched");
        drop(remade);
        assert!(folder.is_dir(), "the new holder's lock is kept");
        fs::remove_dir(&folder).unwrap();

        let touched = ConfigLock::take(&config, &quick(), &uncancelled).unwrap();
        let theirs = SystemTime::now() + LONG_AGO;
        touch(&folder, theirs);
        drop(touched);
        assert!(folder.is_dir(), "a lock touched by another is kept");
        assert_eq!(modified(&folder), Some(theirs));
    }
}
