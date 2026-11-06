use std::{
    ffi::OsString,
    fs::{self, File},
    io,
    path::{Path, PathBuf},
    thread::{self, JoinHandle},
    time::{Duration, SystemTime},
};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, bounded};

use super::{
    agent_config_error::AgentConfigError, config_file::io_error, lock_timing::LockTiming,
    lock_token::LockToken,
};

/// What Claude Code appends to its config's path for the folder that locks
/// it, as its `proper-lockfile` lock names it.
const LOCK_SUFFIX: &str = ".lock";
/// What a failed lock was doing.
const LOCK: &str = "lock";
/// The name of the thread keeping a held lock fresh.
const REFRESH_THREAD: &str = "config-lock-refresh";

/// Sets a lock folder's modified time to now, so Claude Code never takes
/// it for stale.
type Stamper = fn(&Path) -> io::Result<()>;

/// The lock Claude Code takes around each re-read and write of its config,
/// held by muster across its own: an empty folder beside the file, made
/// with `mkdir`, muster's while the token file beside it is muster's and
/// names that very folder, stamped while held so it never looks stale, and
/// removed when the guard drops only while it is muster's still. An RAII
/// guard, so it has a hand-written constructor.
#[derive(Debug)]
pub(super) struct ConfigLock {
    /// The config it locks, as the agent names it.
    config: PathBuf,
    /// The lock folder.
    folder: PathBuf,
    /// muster's claim on the folder, kept beside it.
    token: LockToken,
    /// Dropped to stop the refresh.
    stop: Option<Sender<()>>,
    /// The refresh.
    refresh: Option<JoinHandle<()>>,
}

impl ConfigLock {
    /// Takes the lock of `config`, the config's path as Claude Code is
    /// configured with it, links not followed, as Claude Code names its own
    /// lock, waiting while another writer holds it and taking over one gone
    /// stale, as `timing` says. Without the config's folder there is
    /// nothing to lock, and none is taken.
    ///
    /// # Errors
    /// Returns [`AgentConfigError::Locked`] when the lock stays held,
    /// [`AgentConfigError::Interrupted`] when `cancelled` says so while
    /// waiting, or [`AgentConfigError::Io`] when the folder or its token
    /// cannot be made.
    pub(super) fn take(
        config: &Path,
        timing: &LockTiming,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Self>, AgentConfigError> {
        Self::take_stamped(config, timing, cancelled, stamp)
    }

    /// As [`Self::take`], the folder stamped by `stamper`.
    ///
    /// # Errors
    /// As [`Self::take`].
    fn take_stamped(
        config: &Path,
        timing: &LockTiming,
        cancelled: &dyn Fn() -> bool,
        stamper: Stamper,
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
                Ok(()) => {
                    return Self::hold(config, folder, timing.refresh_every(), stamper).map(Some);
                },
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

    /// Checks the lock is muster's still, so nothing is written under a
    /// lock another writer took over.
    ///
    /// # Errors
    /// Returns [`AgentConfigError::LockLost`] when it is not.
    pub(super) fn check_held(&self) -> Result<(), AgentConfigError> {
        if self.token.holds(&self.folder) {
            Ok(())
        } else {
            Err(AgentConfigError::LockLost(self.config.clone()))
        }
    }

    /// The guard of `folder`, just made for `config`: claimed by a token
    /// beside it, stamped by `stamper`, and stamped again every
    /// `refresh_every` from a thread of its own.
    ///
    /// # Errors
    /// Returns [`AgentConfigError::Locked`] when another writer made a token
    /// file beside the folder first, leaving both to it, or
    /// [`AgentConfigError::Io`] when the token cannot be written, the folder
    /// stamped or the thread started; the lock is let go again.
    fn hold(
        config: &Path,
        folder: PathBuf,
        refresh_every: Duration,
        stamper: Stamper,
    ) -> Result<Self, AgentConfigError> {
        let token = match LockToken::claim(&folder) {
            Ok(token) => token,
            // The folder may be the other writer's by now, so it stays; an
            // empty one muster left goes stale like any other.
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                return Err(AgentConfigError::Locked(config.to_path_buf()));
            },
            Err(source) => {
                let _ = fs::remove_dir(&folder);
                return Err(io_error(LOCK, config, source));
            },
        };
        let start = || -> io::Result<_> {
            stamper(&folder)?;
            let (stop, stopped) = bounded(0);
            let (watched, ours) = (folder.clone(), token.clone());
            let refresh = thread::Builder::new()
                .name(REFRESH_THREAD.to_owned())
                .spawn(move || refresh(&watched, &ours, refresh_every, &stopped, stamper))?;
            Ok((stop, refresh))
        };
        match start() {
            Ok((stop, refresh)) => Ok(Self {
                config: config.to_path_buf(),
                folder,
                token,
                stop: Some(stop),
                refresh: Some(refresh),
            }),
            Err(source) => {
                token.release(&folder);
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
        if let Some(refresh) = self.refresh.take() {
            let _ = refresh.join();
        }
        self.token.release(&self.folder);
    }
}

/// Stamps `folder` with `stamper` every `every` while `token` holds it,
/// until `stopped` says to stop. A stamp that fails is tried again at the
/// next turn, well within the stale bound; only a lock no longer muster's
/// ends the refresh, leaving it to its new holder.
fn refresh(
    folder: &Path,
    token: &LockToken,
    every: Duration,
    stopped: &Receiver<()>,
    stamper: Stamper,
) {
    while let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(every) {
        if !token.holds(folder) {
            return;
        }
        let _ = stamper(folder);
    }
}

/// Sets `folder`'s modified time to now.
///
/// # Errors
/// Returns the failure to open the folder or set the time.
fn stamp(folder: &Path) -> io::Result<()> {
    File::open(folder)?.set_modified(SystemTime::now())
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
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::Instant,
    };

    use tempfile::TempDir;

    use super::*;
    use crate::adapter::agent_config::{lock_token::token_file, testing::uncancelled};

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
    /// How many refreshes a test lets go by.
    const REFRESHES: u32 = 4;

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

    /// Takes the lock of `config` with `timing`.
    fn take(config: &Path, timing: &LockTiming) -> Result<Option<ConfigLock>, AgentConfigError> {
        ConfigLock::take(config, timing, &uncancelled)
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
        let lock = take(&config, &claude()).unwrap();
        assert!(lock.is_some() && folder.is_dir());
        drop(lock);
        assert!(!folder.exists());
        let homeless = dir.path().join("missing/.claude.json");
        assert!(take(&homeless, &claude()).unwrap().is_none());
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
        let lock = take(&config, &claude()).unwrap();
        assert!(started.elapsed() >= HELD_FOR);
        assert!(lock.is_some() && folder.is_dir());
        release.join().unwrap();
    }

    /// A lock untouched past the stale bound was left by a writer that
    /// died, so it is taken over, even one a muster that died left with its
    /// token beside it; an interrupt while waiting gives up.
    #[test]
    fn a_stale_lock_is_taken_and_an_interrupt_stops_the_wait() {
        let dir = TempDir::new().unwrap();
        let (config, folder) = paths(&dir);
        for dead_muster in [false, true] {
            fs::create_dir(&folder).unwrap();
            if dead_muster {
                LockToken::claim(&folder).unwrap();
            }
            touch(&folder, SystemTime::now() - LONG_AGO);
            let started = Instant::now();
            let lock = take(&config, &claude()).unwrap();
            assert!(lock.is_some(), "dead muster {dead_muster}");
            assert!(
                started.elapsed() < claude().retry_pauses()[0],
                "taken without a wait"
            );
            drop(lock);
            assert!(!folder.exists() && !token_file(&folder).exists());
        }

        fs::create_dir(&folder).unwrap();
        assert!(matches!(
            ConfigLock::take(&config, &claude(), &|| true),
            Err(AgentConfigError::Interrupted)
        ));
        assert!(folder.is_dir(), "another writer's lock is left alone");
    }

    /// A muster that dies holding the lock leaves its folder empty, so
    /// Claude Code's `rmdir` takes it over once stale; the token left
    /// beside it holds nothing, and the next run of muster replaces it.
    #[test]
    fn a_lock_left_by_a_crash_is_taken_over_by_claude_code() {
        let dir = TempDir::new().unwrap();
        let (config, folder) = paths(&dir);
        let held = take(&config, &claude()).unwrap().unwrap();
        let dead = held.token.clone();
        // A crash runs no drop: the folder and the token beside it stay.
        let ConfigLock { stop, refresh, .. } = &mut *Box::leak(Box::new(held));
        drop(stop.take());
        refresh.take().unwrap().join().unwrap();
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 0, "kept empty");

        touch(&folder, SystemTime::now() - LONG_AGO);
        fs::remove_dir(&folder).expect("Claude Code's stale takeover");
        fs::create_dir(&folder).unwrap();
        assert!(!dead.holds(&folder), "Claude Code's folder is not muster's");
        fs::remove_dir(&folder).unwrap();

        let next = take(&config, &claude()).unwrap().unwrap();
        drop(next);
        assert!(!folder.exists() && !token_file(&folder).exists());
    }

    /// A lock muster holds past the stale bound stays fresh, so another
    /// writer waits it out rather than taking it over.
    #[test]
    fn a_hold_past_the_stale_bound_is_not_taken_over() {
        let dir = TempDir::new().unwrap();
        let (config, folder) = paths(&dir);
        let held = take(&config, &quick()).unwrap().unwrap();
        thread::sleep(QUICK_STALE + PAST_STALE);
        assert!(!is_stale(&folder, QUICK_STALE), "kept fresh");
        assert!(matches!(
            take(&config, &quick()),
            Err(AgentConfigError::Locked(_))
        ));
        assert!(held.check_held().is_ok(), "still muster's");
        drop(held);
        assert!(!folder.exists());
    }

    /// A lock another writer took over, by removing the folder and making
    /// it again as Claude Code does or by leaving its own token beside it
    /// as another muster does, is neither stamped again nor removed by
    /// muster, and reads as lost; muster's own token beside a folder made
    /// again goes. A lock merely touched by another is muster's still.
    #[test]
    fn a_lock_taken_over_is_left_to_its_new_holder() {
        let dir = TempDir::new().unwrap();
        let (config, folder) = paths(&dir);
        let token = token_file(&folder);
        for by_another_muster in [false, true] {
            let lock = take(&config, &quick()).unwrap().unwrap();
            if by_another_muster {
                fs::write(&token, "theirs").unwrap();
            } else {
                fs::remove_dir(&folder).unwrap();
                fs::create_dir(&folder).unwrap();
            }
            let theirs = SystemTime::now() - QUICK_REFRESH;
            touch(&folder, theirs);
            assert!(matches!(
                lock.check_held(),
                Err(AgentConfigError::LockLost(_))
            ));
            thread::sleep(QUICK_REFRESH * 2);
            assert_eq!(modified(&folder), Some(theirs), "no longer stamped");
            drop(lock);
            assert!(folder.is_dir(), "the new holder's lock is kept");
            assert_eq!(token.exists(), by_another_muster);
            fs::remove_dir(&folder).unwrap();
            let _ = fs::remove_file(&token);
        }

        let touched = take(&config, &quick()).unwrap().unwrap();
        touch(&folder, SystemTime::now() + LONG_AGO);
        assert!(touched.check_held().is_ok(), "a touch takes nothing over");
        drop(touched);
        assert!(!folder.exists());
    }

    /// How many stamps the flaky stamper has been asked for.
    static FLAKY_CALLS: AtomicUsize = AtomicUsize::new(0);

    /// Stamps as usual but fails the second and third time, as a file
    /// system that refuses a write for a moment does.
    fn flaky(folder: &Path) -> io::Result<()> {
        match FLAKY_CALLS.fetch_add(1, Ordering::SeqCst) {
            1 | 2 => Err(io::Error::other("busy")),
            _ => stamp(folder),
        }
    }

    /// A stamp that fails for a moment is tried again, and the lock stays
    /// muster's and is removed when let go rather than stranded.
    #[test]
    fn a_failed_stamp_does_not_strand_the_lock() {
        let dir = TempDir::new().unwrap();
        let (config, folder) = paths(&dir);
        let held = ConfigLock::take_stamped(&config, &quick(), &uncancelled, flaky)
            .unwrap()
            .unwrap();
        thread::sleep(QUICK_REFRESH * (REFRESHES + 1));
        assert!(FLAKY_CALLS.load(Ordering::SeqCst) > 3, "stamped again");
        assert!(!is_stale(&folder, QUICK_REFRESH * 2), "kept fresh");
        assert!(held.check_held().is_ok());
        drop(held);
        assert!(!folder.exists(), "no lock is left behind");
    }
}
