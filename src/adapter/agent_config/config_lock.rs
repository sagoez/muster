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
    agent_config_error::AgentConfigError,
    config_file::io_error,
    lock_timing::LockTiming,
    lock_token::{LockToken, clear},
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
/// held by muster across its own: a folder beside the file, made with
/// `mkdir`, muster's while it holds muster's token, stamped while held so it
/// never looks stale, and removed when the guard drops only while it holds
/// muster's token still. An RAII guard, so it has a hand-written
/// constructor.
#[derive(Debug)]
pub(super) struct ConfigLock {
    /// The config it locks, as the agent names it.
    config: PathBuf,
    /// The lock folder.
    folder: PathBuf,
    /// The token muster left in the folder.
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
                clear(&folder);
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

    /// Checks the lock is muster's still, its folder holding muster's
    /// token, so nothing is written under a lock another writer took over.
    ///
    /// # Errors
    /// Returns [`AgentConfigError::LockLost`] when it is not.
    pub(super) fn check_held(&self) -> Result<(), AgentConfigError> {
        if self.token.is_in(&self.folder) {
            Ok(())
        } else {
            Err(AgentConfigError::LockLost(self.config.clone()))
        }
    }

    /// The guard of `folder`, just made for `config`: given muster's token,
    /// stamped by `stamper`, and stamped again every `refresh_every` from a
    /// thread of its own.
    ///
    /// # Errors
    /// Returns [`AgentConfigError::Io`] when the token cannot be left, the
    /// folder stamped or the thread started; the folder is removed again.
    fn hold(
        config: &Path,
        folder: PathBuf,
        refresh_every: Duration,
        stamper: Stamper,
    ) -> Result<Self, AgentConfigError> {
        let token = LockToken::fresh();
        let start = || -> io::Result<_> {
            token.leave_in(&folder)?;
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
                // The token may not have been left, so the folder just made
                // is cleared whatever it holds.
                clear(&folder);
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

/// Stamps `folder` with `stamper` every `every` while it holds `token`,
/// until `stopped` says to stop. A stamp that fails is tried again at the
/// next turn, well within the stale bound; only a folder no longer holding
/// the token ends the refresh, leaving it to its new holder.
fn refresh(
    folder: &Path,
    token: &LockToken,
    every: Duration,
    stopped: &Receiver<()>,
    stamper: Stamper,
) {
    while let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(every) {
        if !token.is_in(folder) {
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
    use crate::adapter::agent_config::{lock_token::TOKEN_FILE, testing::uncancelled};

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
    /// died, so it is taken over, even one holding the token of a muster
    /// that died; an interrupt while waiting gives up.
    #[test]
    fn a_stale_lock_is_taken_and_an_interrupt_stops_the_wait() {
        let dir = TempDir::new().unwrap();
        let (config, folder) = paths(&dir);
        for dead_token in [false, true] {
            fs::create_dir(&folder).unwrap();
            if dead_token {
                fs::write(folder.join(TOKEN_FILE), "gone").unwrap();
            }
            touch(&folder, SystemTime::now() - LONG_AGO);
            let started = Instant::now();
            let lock = take(&config, &claude()).unwrap();
            assert!(lock.is_some(), "dead token {dead_token}");
            assert!(
                started.elapsed() < claude().retry_pauses()[0],
                "taken without a wait"
            );
            drop(lock);
            assert!(!folder.exists());
        }

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

    /// A lock another writer took over by removing the folder and making
    /// it again is neither stamped again nor removed by muster, and reads
    /// as lost; so is one whose new folder reuses the old one's inode, as
    /// the folder emptied in place and stamped just now stands for. A lock
    /// merely touched by another is muster's still.
    #[test]
    fn a_lock_taken_over_is_left_to_its_new_holder() {
        let dir = TempDir::new().unwrap();
        let (config, folder) = paths(&dir);
        let remake = |folder: &Path| {
            clear(folder);
            fs::create_dir(folder).unwrap();
        };
        let reuse_inode = |folder: &Path| {
            fs::remove_file(folder.join(TOKEN_FILE)).unwrap();
        };
        let takeovers: [&dyn Fn(&Path); 2] = [&remake, &reuse_inode];
        for take_over in takeovers {
            let lock = take(&config, &quick()).unwrap().unwrap();
            take_over(&folder);
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
            fs::remove_dir(&folder).unwrap();
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
