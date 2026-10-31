use std::{
    ffi::OsString,
    fs::{self, File},
    io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    thread::{self, JoinHandle},
    time::{Duration, SystemTime},
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
/// How far the lock folder's modified time may read back from the time
/// muster set on it and still be muster's: FAT and exFAT keep it in steps
/// of 2 seconds, and network storage may round it as coarsely.
const STAMP_TOLERANCE: Duration = Duration::from_secs(2);

/// The lock Claude Code takes around each re-read and write of its config,
/// held by muster across its own: a folder beside the file, made with
/// `mkdir`, touched while held so it never looks stale, and removed when the
/// guard drops only if it is still muster's. An RAII guard, so it has a
/// hand-written constructor.
#[derive(Debug)]
pub(super) struct ConfigLock {
    /// The config it locks.
    config: PathBuf,
    /// The lock folder.
    folder: PathBuf,
    /// The folder's identity and the last time muster set on it, or none
    /// once another writer took the lock over; shared with the refresh.
    held: Arc<Mutex<Option<Stamp>>>,
    /// Dropped to stop the refresh.
    stop: Option<Sender<()>>,
    /// The refresh.
    refresh: Option<JoinHandle<()>>,
}

/// The lock folder muster made, and the time it last set on it.
#[derive(Clone, Copy, Debug)]
struct Stamp {
    /// The device holding the folder.
    device: u64,
    /// The folder's inode, which a folder made again in its place replaces.
    inode: u64,
    /// The modified time muster last set.
    at: SystemTime,
}

impl ConfigLock {
    /// Takes the lock of `config`, the config's fully resolved path, as
    /// Claude Code's lock resolves it, waiting while another writer holds it
    /// and taking over one gone stale, as `timing` says. Without the
    /// config's folder there is nothing to lock, and none is taken.
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
                Ok(()) => return Self::hold(config, folder, timing.refresh_every()).map(Some),
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

    /// Checks the lock is muster's still, so nothing is written under a lock
    /// another writer took over.
    ///
    /// # Errors
    /// Returns [`AgentConfigError::LockLost`] when it is not.
    pub(super) fn check_held(&self) -> Result<(), AgentConfigError> {
        // The guard stays held through the look, so a refresh cannot stamp
        // the folder anew in between.
        let held = guarded(&self.held);
        if held.is_some_and(|stamp| bears(&self.folder, &stamp)) {
            Ok(())
        } else {
            Err(AgentConfigError::LockLost(self.config.clone()))
        }
    }

    /// The guard of `folder`, just made for `config`: stamped as muster's
    /// and touched every `refresh_every` from a thread of its own.
    ///
    /// # Errors
    /// Returns [`AgentConfigError::Io`] when the folder cannot be stamped or
    /// the thread started; the folder is removed again.
    fn hold(
        config: &Path,
        folder: PathBuf,
        refresh_every: Duration,
    ) -> Result<Self, AgentConfigError> {
        let start = || -> io::Result<_> {
            let held = Arc::new(Mutex::new(Some(stamp(&folder)?)));
            let (stop, stopped) = bounded(0);
            let (watched, shared) = (folder.clone(), Arc::clone(&held));
            let refresh = thread::Builder::new()
                .name(REFRESH_THREAD.to_owned())
                .spawn(move || refresh(&watched, &shared, refresh_every, &stopped))?;
            Ok((held, stop, refresh))
        };
        match start() {
            Ok((held, stop, refresh)) => Ok(Self {
                config: config.to_path_buf(),
                folder,
                held,
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
        if let Some(refresh) = self.refresh.take() {
            let _ = refresh.join();
        }
        if guarded(&self.held)
            .take()
            .is_some_and(|stamp| bears(&self.folder, &stamp))
        {
            let _ = fs::remove_dir(&self.folder);
        }
    }
}

/// Touches `folder` every `every` while it still bears the stamp `held`
/// keeps, until `stopped` says to stop; once it does not, or cannot be
/// touched, `held` is cleared and the folder left to its new holder.
fn refresh(folder: &Path, held: &Mutex<Option<Stamp>>, every: Duration, stopped: &Receiver<()>) {
    while let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(every) {
        let mut held = guarded(held);
        let restamped = (*held)
            .filter(|stamp| bears(folder, stamp))
            .and_then(|_| stamp(folder).ok());
        *held = restamped;
        if restamped.is_none() {
            return;
        }
    }
}

/// `held`, locked; a refresh that panicked leaves the stamp as it was.
fn guarded(held: &Mutex<Option<Stamp>>) -> MutexGuard<'_, Option<Stamp>> {
    held.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Sets `folder`'s modified time to now and returns it with the folder's
/// identity.
///
/// # Errors
/// Returns the failure to set it or look at the folder.
fn stamp(folder: &Path) -> io::Result<Stamp> {
    let at = SystemTime::now();
    let file = File::open(folder)?;
    file.set_modified(at)?;
    let metadata = file.metadata()?;
    Ok(Stamp {
        device: metadata.dev(),
        inode: metadata.ino(),
        at,
    })
}

/// Whether `folder` is still the one `stamp` was set on, with a modified
/// time within [`STAMP_TOLERANCE`] of it, however coarsely its file system
/// keeps times.
fn bears(folder: &Path, stamp: &Stamp) -> bool {
    fs::metadata(folder).is_ok_and(|metadata| {
        metadata.dev() == stamp.device
            && metadata.ino() == stamp.inode
            && metadata.modified().is_ok_and(|modified| {
                let drift = modified
                    .duration_since(stamp.at)
                    .or_else(|_| stamp.at.duration_since(modified));
                drift.is_ok_and(|drift| drift <= STAMP_TOLERANCE)
            })
    })
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
        sync::atomic::{AtomicBool, Ordering},
        time::{Instant, UNIX_EPOCH},
    };

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
    /// The step a FAT file system keeps modified times in.
    const FAT_STEP: u64 = 2;
    /// How often the simulated coarse file system rounds the lock's time.
    const COARSE_EVERY: Duration = Duration::from_millis(20);
    /// How many refreshes a test holds a lock through on a coarse clock.
    const COARSE_REFRESHES: u32 = 5;

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

    /// Rounds `folder`'s modified time down to FAT's 2-second step, as a
    /// FAT file system keeps whatever time is set on it.
    fn round_to_fat(folder: &Path) {
        let Some(at) = modified(folder) else {
            return;
        };
        let seconds = at.duration_since(UNIX_EPOCH).unwrap().as_secs();
        let _ = File::open(folder).and_then(|file| {
            file.set_modified(UNIX_EPOCH + Duration::from_secs(seconds / FAT_STEP * FAT_STEP))
        });
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
        let held = ConfigLock::take(&config, &quick(), &uncancelled)
            .unwrap()
            .unwrap();
        thread::sleep(QUICK_STALE + PAST_STALE);
        assert!(!is_stale(&folder, QUICK_STALE), "kept fresh");
        assert!(matches!(
            ConfigLock::take(&config, &quick(), &uncancelled),
            Err(AgentConfigError::Locked(_))
        ));
        assert!(held.check_held().is_ok(), "still muster's");
        drop(held);
        assert!(!folder.exists());
    }

    /// A lock another writer took over, by making the folder again or
    /// touching it, is neither touched again nor removed by muster, and
    /// reads as lost.
    #[test]
    fn a_lock_taken_over_is_left_to_its_new_holder() {
        let dir = TempDir::new().unwrap();
        let (config, folder) = paths(&dir);

        let remade = ConfigLock::take(&config, &quick(), &uncancelled)
            .unwrap()
            .unwrap();
        // Moved aside before it goes, so the new folder cannot reuse its
        // inode.
        let aside = dir.path().join("aside");
        fs::rename(&folder, &aside).unwrap();
        fs::create_dir(&folder).unwrap();
        fs::remove_dir(&aside).unwrap();
        let theirs = modified(&folder);
        thread::sleep(QUICK_REFRESH * 3);
        assert_eq!(modified(&folder), theirs, "no longer touched");
        assert!(matches!(
            remade.check_held(),
            Err(AgentConfigError::LockLost(_))
        ));
        drop(remade);
        assert!(folder.is_dir(), "the new holder's lock is kept");
        fs::remove_dir(&folder).unwrap();

        let touched = ConfigLock::take(&config, &quick(), &uncancelled)
            .unwrap()
            .unwrap();
        let theirs = SystemTime::now() + LONG_AGO;
        touch(&folder, theirs);
        assert!(touched.check_held().is_err());
        drop(touched);
        assert!(folder.is_dir(), "a lock touched by another is kept");
        assert_eq!(modified(&folder), Some(theirs));
    }

    /// On a file system that keeps times in 2-second steps, the lock read
    /// back is muster's still through every refresh, and is removed when
    /// let go rather than left behind.
    #[test]
    fn a_coarse_clock_leaves_no_lock_behind() {
        let dir = TempDir::new().unwrap();
        let (config, folder) = paths(&dir);
        let held = ConfigLock::take(&config, &quick(), &uncancelled)
            .unwrap()
            .unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let coarse = {
            let (folder, done) = (folder.clone(), Arc::clone(&done));
            thread::spawn(move || {
                while !done.load(Ordering::Relaxed) {
                    round_to_fat(&folder);
                    thread::sleep(COARSE_EVERY);
                }
            })
        };
        thread::sleep(QUICK_REFRESH * COARSE_REFRESHES);
        let still = held.check_held();
        done.store(true, Ordering::Relaxed);
        coarse.join().unwrap();
        assert!(still.is_ok(), "{still:?}");
        round_to_fat(&folder);
        drop(held);
        assert!(!folder.exists(), "no lock is left behind");
    }
}
