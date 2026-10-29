use std::{
    fmt::Display,
    fs::{OpenOptions, Permissions},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, ErrorCode, TransactionBehavior};

use crate::{
    adapter::private_dir::{prepare_state_dir, state_dir},
    domain::{config::ConfigError, value::PositiveDuration},
};

/// Nanoseconds in a millisecond, for rounding a grace period up.
const NANOS_PER_MILLI: u128 = 1_000_000;
/// The database file under muster's state directory.
pub const DATABASE_FILE: &str = "muster.db";
/// How long a connection waits for another process's write lock before
/// failing.
const BUSY_TIMEOUT: Duration = Duration::from_millis(5000);
/// Pause before switching a fresh file to WAL again after another opener
/// held its lock.
const BUSY_RETRY: Duration = Duration::from_millis(10);
/// Mode of the database file, which SQLite gives its `-wal` and `-shm` too:
/// its owner's alone.
const PRIVATE_DB_MODE: u32 = 0o600;
/// Pragmas every connection runs once the busy timeout is set. WAL lets the
/// service read while a CLI or MCP process writes.
const CONNECTION_PRAGMAS: &str = "\
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;";
/// The schema pragma recording how many migrations have been applied.
const USER_VERSION: &str = "user_version";
/// Every schema migration, in order. A shipped migration is never edited; a
/// change is a new file appended here.
const MIGRATIONS: &[&str] = &[include_str!("migrations/0001_initial.sql")];

/// The one SQLite database holding projects, processes, settings and
/// coordination. Clones share one connection; every write is one
/// `BEGIN IMMEDIATE` transaction, so writers in other processes serialize on
/// SQLite's lock.
#[derive(Clone)]
pub struct SqliteStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteStore {
    /// Opens the database in muster's state directory, creating it if absent.
    ///
    /// # Errors
    /// Returns [`ConfigError::NoStateDir`] without a state directory, or what
    /// [`SqliteStore::open`] returns.
    pub fn open_default() -> Result<Self, ConfigError> {
        let dir = state_dir().ok_or(ConfigError::NoStateDir)?;
        Self::open(&dir.join(DATABASE_FILE))
    }

    /// Opens (creating if needed) the database at `path` and migrates it to the
    /// latest schema. The parent directory is readied owner-only and the file
    /// is created 0600.
    ///
    /// # Errors
    /// Returns [`ConfigError::Write`] when the directory or file cannot be
    /// prepared, [`ConfigError::UnsupportedSchema`] for a database a newer
    /// muster wrote, or [`ConfigError::Database`] when SQLite fails.
    pub fn open(path: &Path) -> Result<Self, ConfigError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            prepare_state_dir(parent).map_err(|source| ConfigError::Write {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        OpenOptions::new()
            .create(true)
            .append(true)
            .mode(PRIVATE_DB_MODE)
            .open(path)
            .and_then(|file| file.set_permissions(Permissions::from_mode(PRIVATE_DB_MODE)))
            .map_err(|source| ConfigError::Write {
                path: path.to_path_buf(),
                source,
            })?;
        let mut conn = Connection::open(path).map_err(db_error)?;
        conn.busy_timeout(BUSY_TIMEOUT).map_err(db_error)?;
        switch_to_wal(&conn).map_err(db_error)?;
        migrate(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Runs `work` in a read transaction, so its queries see one snapshot.
    ///
    /// # Errors
    /// Returns what `work` returns, or [`ConfigError::Database`] when the
    /// transaction cannot begin.
    pub(super) fn read<T>(
        &self,
        work: impl FnOnce(&Connection) -> Result<T, ConfigError>,
    ) -> Result<T, ConfigError> {
        let mut conn = self.lock()?;
        let transaction = conn.transaction().map_err(db_error)?;
        work(&transaction)
    }

    /// Runs `work` in one `BEGIN IMMEDIATE` transaction, committing when it
    /// succeeds and rolling back otherwise.
    ///
    /// # Errors
    /// Returns what `work` returns, or [`ConfigError::Database`] when the
    /// transaction cannot begin or commit.
    pub(super) fn write<T>(
        &self,
        work: impl FnOnce(&Connection) -> Result<T, ConfigError>,
    ) -> Result<T, ConfigError> {
        let mut conn = self.lock()?;
        let transaction = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let value = work(&transaction)?;
        transaction.commit().map_err(db_error)?;
        Ok(value)
    }

    /// Locks the shared connection for one operation.
    ///
    /// # Errors
    /// Returns [`ConfigError::Database`] when a previous holder panicked.
    fn lock(&self) -> Result<MutexGuard<'_, Connection>, ConfigError> {
        self.conn
            .lock()
            .map_err(|_| ConfigError::Database("the database connection lock is poisoned".into()))
    }
}

/// Applies every pending migration in one immediate transaction, so two
/// processes opening a fresh database serialize and the second sees the
/// version the first wrote.
///
/// # Errors
/// Returns [`ConfigError::UnsupportedSchema`] when the database is newer than
/// [`MIGRATIONS`], or [`ConfigError::Database`] when a migration fails.
fn migrate(conn: &mut Connection) -> Result<(), ConfigError> {
    let latest = MIGRATIONS.len();
    let transaction = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_error)?;
    let found: u32 = transaction
        .pragma_query_value(None, USER_VERSION, |row| row.get(0))
        .map_err(db_error)?;
    let applied = found as usize;
    if applied > latest {
        return Err(ConfigError::UnsupportedSchema {
            found,
            supported: latest as u32,
        });
    }
    for (index, migration) in MIGRATIONS.iter().enumerate().skip(applied) {
        transaction.execute_batch(migration).map_err(db_error)?;
        transaction
            .pragma_update(None, USER_VERSION, index as i64 + 1)
            .map_err(db_error)?;
    }
    transaction.commit().map_err(db_error)
}

/// Runs [`CONNECTION_PRAGMAS`], retrying while another opener holds the lock
/// for at most [`BUSY_TIMEOUT`]. Two connections switching one fresh file to
/// WAL at once deadlock on its lock, so SQLite fails one of them busy at once
/// rather than calling the busy handler.
///
/// # Errors
/// Returns the driver's error, or the busy error once the time is up.
fn switch_to_wal(conn: &Connection) -> rusqlite::Result<()> {
    let deadline = Instant::now() + BUSY_TIMEOUT;
    loop {
        match conn.execute_batch(CONNECTION_PRAGMAS) {
            Err(error)
                if error.sqlite_error_code() == Some(ErrorCode::DatabaseBusy)
                    && Instant::now() < deadline =>
            {
                thread::sleep(BUSY_RETRY);
            },
            outcome => return outcome,
        }
    }
}

/// Maps a driver error to the domain error, keeping `rusqlite` out of the
/// domain.
pub(super) fn db_error(error: rusqlite::Error) -> ConfigError {
    ConfigError::Database(error.to_string())
}

/// The error for a stored value the domain refuses: the row was edited by
/// hand or written by a different muster.
pub(super) fn corrupt(what: &str, error: impl Display) -> ConfigError {
    ConfigError::Database(format!("stored {what} is invalid: {error}"))
}

/// A grace period as whole milliseconds, rounded up so a positive grace never
/// stores as zero.
pub(super) fn grace_millis(grace: PositiveDuration) -> i64 {
    grace.into_inner().as_nanos().div_ceil(NANOS_PER_MILLI) as i64
}

/// The grace period a millisecond column holds.
///
/// # Errors
/// Returns [`ConfigError::Database`] naming `what` when it is not positive.
pub(super) fn grace_from_millis(what: &str, millis: i64) -> Result<PositiveDuration, ConfigError> {
    u64::try_from(millis)
        .map_err(|error| corrupt(what, error))
        .and_then(|millis| {
            PositiveDuration::try_new(Duration::from_millis(millis))
                .map_err(|error| corrupt(what, error))
        })
}

/// Seconds since the Unix epoch, or zero if the clock predates it. The store
/// stamps write times so the domain need not depend on a clock.
pub(super) fn now_epoch_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::{fs, thread};

    use tempfile::TempDir;

    use super::*;
    use crate::domain::port::SettingsStore;

    /// Mode bits a permission check compares.
    const PERMISSION_BITS: u32 = 0o777;
    /// The mode of a directory only its owner can enter.
    const OWNER_DIR: u32 = 0o700;
    /// A directory mode other users can enter.
    const SHARED_DIR: u32 = 0o755;
    /// Suffixes of the files SQLite keeps beside a WAL database.
    const SIDE_FILES: [&str; 2] = ["-wal", "-shm"];
    /// Openers racing on one fresh database.
    const RACING_OPENERS: usize = 8;

    fn mode_of(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & PERMISSION_BITS
    }

    fn user_version(path: &Path) -> u32 {
        Connection::open(path)
            .unwrap()
            .pragma_query_value(None, USER_VERSION, |row| row.get(0))
            .unwrap()
    }

    /// The database, its WAL and shared-memory files, and the state directory
    /// are their owner's alone, whether the directory is created fresh or
    /// found open to other users.
    #[test]
    fn the_database_its_side_files_and_directory_are_private() {
        for existing in [false, true] {
            let scratch = TempDir::new().unwrap();
            let dir = scratch.path().join("muster");
            if existing {
                fs::create_dir(&dir).unwrap();
                fs::set_permissions(&dir, Permissions::from_mode(SHARED_DIR)).unwrap();
            }
            let path = dir.join(DATABASE_FILE);

            let store = SqliteStore::open(&path).unwrap();
            store.load().unwrap();

            assert_eq!(mode_of(&dir), OWNER_DIR, "existing: {existing}");
            assert_eq!(mode_of(&path), PRIVATE_DB_MODE, "existing: {existing}");
            for suffix in SIDE_FILES {
                let side = dir.join(format!("{DATABASE_FILE}{suffix}"));
                assert_eq!(mode_of(&side), PRIVATE_DB_MODE, "{}", side.display());
            }
        }
    }

    /// A fresh database migrates to the latest version; reopening applies
    /// nothing twice.
    #[test]
    fn migrates_from_empty_and_reopens_idempotently() {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join(DATABASE_FILE);

        SqliteStore::open(&path).unwrap();
        assert_eq!(user_version(&path), MIGRATIONS.len() as u32);
        let store = SqliteStore::open(&path).unwrap();

        assert_eq!(user_version(&path), MIGRATIONS.len() as u32);
        assert!(store.load().is_ok(), "the seeded settings row survives");
    }

    /// A database a newer muster wrote is refused, and left untouched.
    #[test]
    fn refuses_a_newer_schema() {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join(DATABASE_FILE);
        let newer = MIGRATIONS.len() as u32 + 1;
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, USER_VERSION, newer)
            .unwrap();

        let error = SqliteStore::open(&path).err().unwrap();

        assert!(matches!(
            error,
            ConfigError::UnsupportedSchema { found, supported }
                if found == newer && supported == MIGRATIONS.len() as u32
        ));
        assert_eq!(user_version(&path), newer);
    }

    /// A sub-millisecond grace is stored rounded up, never as zero, and a
    /// non-positive stored grace is refused.
    #[test]
    fn a_grace_rounds_up_to_whole_milliseconds() {
        let grace = |nanos| PositiveDuration::try_new(Duration::from_nanos(nanos)).unwrap();

        assert_eq!(grace_millis(grace(1_500_000)), 2);
        assert_eq!(grace_millis(grace(1)), 1);
        assert_eq!(
            grace_from_millis("grace", 5000).unwrap(),
            grace(5_000_000_000)
        );
        assert!(grace_from_millis("grace", 0).is_err());
        assert!(grace_from_millis("grace", -1).is_err());
    }

    /// Processes opening one fresh database at once all succeed, and the
    /// migration runs exactly once.
    #[test]
    fn concurrent_first_opens_migrate_once() {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join(DATABASE_FILE);

        let openers: Vec<_> = (0..RACING_OPENERS)
            .map(|_| {
                let path = path.clone();
                thread::spawn(move || SqliteStore::open(&path).map(|_| ()))
            })
            .collect();
        for opener in openers {
            opener.join().unwrap().unwrap();
        }

        assert_eq!(user_version(&path), MIGRATIONS.len() as u32);
        let rows: i64 = Connection::open(&path)
            .unwrap()
            .query_row("SELECT count(*) FROM settings", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }
}
