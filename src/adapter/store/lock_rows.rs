use rusqlite::{Connection, OptionalExtension, Row, named_params, params};

use super::{
    coordination_rows::{author_from, collect},
    sqlite_store::{corrupt, db_error, epoch_from_secs, epoch_param},
    store_error::StoreError,
};
use crate::domain::{
    config::ConfigError,
    coordination::{
        LOCK_TTL_MAX, LOCKS_PER_SESSION_MAX, Lock, LockAttempt, LockHolder, LockName, LockNote,
        LockRelease, LockTtl, SessionId,
    },
    value::EpochSeconds,
};

/// The columns [`lock_from_row`] reads, in order.
const LOCK_COLUMNS: &str = "name, owner, note, expires_at";

/// Takes the lock for `me` when it is free, expired or already `me`'s through
/// the same session, in one upsert whose `RETURNING` yields no row when
/// someone else holds it.
const ACQUIRE: &str = "\
INSERT INTO locks (project_id, name, owner, session, note, expires_at) \
VALUES (:project, :name, :me, :session, :note, :until) \
ON CONFLICT (project_id, name) DO UPDATE SET \
    owner = excluded.owner, session = excluded.session, note = excluded.note, \
    expires_at = excluded.expires_at \
WHERE locks.expires_at < :now \
    OR (locks.owner = excluded.owner AND locks.session = excluded.session) \
RETURNING expires_at";

/// Reads a lock row selected as [`LOCK_COLUMNS`].
fn lock_from_row(row: &Row) -> Result<Lock, ConfigError> {
    let name: String = row.get("name").map_err(db_error)?;
    let owner: String = row.get("owner").map_err(db_error)?;
    let note: Option<String> = row.get("note").map_err(db_error)?;
    let expires_at: i64 = row.get("expires_at").map_err(db_error)?;
    Ok(Lock::builder()
        .name(LockName::try_new(name).map_err(|error| corrupt("lock name", error))?)
        .owner(author_from(&owner)?)
        .note(
            note.map(LockNote::try_new)
                .transpose()
                .map_err(|error| corrupt("lock note", error))?,
        )
        .expires_at(epoch_from_secs("lock expiry", expires_at)?)
        .build())
}

/// The lock `name` of project `project_id`, live or expired.
fn find(conn: &Connection, project_id: i64, name: &LockName) -> Result<Option<Lock>, ConfigError> {
    Ok(collect(
        conn,
        &format!("SELECT {LOCK_COLUMNS} FROM locks WHERE project_id = ?1 AND name = ?2"),
        params![project_id, name.as_ref()],
        lock_from_row,
    )?
    .pop())
}

/// Whether `session` already ended and had its locks released.
fn has_ended(conn: &Connection, session: &SessionId) -> Result<bool, ConfigError> {
    conn.prepare_cached("SELECT 1 FROM ended_sessions WHERE session = ?1")
        .and_then(|mut statement| statement.exists(params![session.as_ref()]))
        .map_err(db_error)
}

/// How many live locks `me`'s session holds at `now` in any project, the
/// lock `name` of project `project_id` aside, so renewing it never counts.
fn other_live_locks(
    conn: &Connection,
    project_id: i64,
    me: &LockHolder,
    name: &LockName,
    now: EpochSeconds,
) -> Result<usize, ConfigError> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM locks WHERE session = :session AND expires_at >= :now \
             AND NOT (project_id = :project AND name = :name)",
            named_params! {
                ":session": me.session().as_ref(),
                ":now": epoch_param(now),
                ":project": project_id,
                ":name": name.as_ref(),
            },
            |row| row.get(0),
        )
        .map_err(db_error)?;
    usize::try_from(count).map_err(|error| corrupt("lock count", error))
}

/// Takes the lock `name` of project `project_id` for `me` for `ttl` at `now`,
/// unless `me`'s session already ended: an acquire still running when its
/// connection closed can then never commit a lock nobody releases.
///
/// # Errors
/// Returns [`ConfigError::TooManyLocks`] when the session already holds
/// [`LOCKS_PER_SESSION_MAX`] other live locks, and a [`ConfigError`] when
/// SQLite fails or a stored row is invalid.
pub(super) fn acquire(
    conn: &Connection,
    project_id: i64,
    me: &LockHolder,
    name: &LockName,
    ttl: LockTtl,
    note: Option<&LockNote>,
    now: EpochSeconds,
) -> Result<LockAttempt, ConfigError> {
    if has_ended(conn, me.session())? {
        return Ok(LockAttempt::SessionEnded);
    }
    if other_live_locks(conn, project_id, me, name, now)? >= LOCKS_PER_SESSION_MAX {
        return Err(ConfigError::TooManyLocks);
    }
    let until = now.after(*ttl.as_ref());
    let taken: Option<i64> = conn
        .query_row(
            ACQUIRE,
            named_params! {
                ":project": project_id,
                ":name": name.as_ref(),
                ":me": me.author().label(),
                ":session": me.session().as_ref(),
                ":note": note.map(AsRef::<str>::as_ref),
                ":until": epoch_param(until),
                ":now": epoch_param(now),
            },
            |row| row.get(0),
        )
        .optional()
        .map_err(db_error)?;
    if taken.is_some() {
        return Ok(LockAttempt::Acquired {
            expires_in: *ttl.as_ref(),
        });
    }
    let holder =
        find(conn, project_id, name)?.ok_or_else(|| StoreError::LockVanished(name.to_string()))?;
    Ok(LockAttempt::HeldBy {
        expires_in: holder.expires_in(now),
        owner: holder.owner().clone(),
        note: holder.note().clone(),
    })
}

/// Releases the lock `name` of project `project_id` when `me` holds it
/// through the same session.
///
/// # Errors
/// Returns a [`ConfigError`] when SQLite fails or a stored row is invalid.
pub(super) fn release(
    conn: &Connection,
    project_id: i64,
    me: &LockHolder,
    name: &LockName,
    now: EpochSeconds,
) -> Result<LockRelease, ConfigError> {
    let removed = conn
        .execute(
            "DELETE FROM locks WHERE project_id = ?1 AND name = ?2 AND owner = ?3 \
             AND session = ?4",
            params![
                project_id,
                name.as_ref(),
                me.author().label(),
                me.session().as_ref()
            ],
        )
        .map_err(db_error)?;
    if removed > 0 {
        return Ok(LockRelease::Released);
    }
    Ok(match find(conn, project_id, name)? {
        Some(lock) if lock.is_live(now) => LockRelease::HeldBy {
            owner: lock.owner().clone(),
        },
        _ => LockRelease::NotHeld,
    })
}

/// Removes the lock `name` of project `project_id` whoever holds it.
///
/// # Errors
/// Returns a [`ConfigError`] when SQLite fails or a stored row is invalid.
pub(super) fn break_lock(
    conn: &Connection,
    project_id: i64,
    name: &LockName,
) -> Result<Option<Lock>, ConfigError> {
    Ok(collect(
        conn,
        &format!("DELETE FROM locks WHERE project_id = ?1 AND name = ?2 RETURNING {LOCK_COLUMNS}"),
        params![project_id, name.as_ref()],
        lock_from_row,
    )?
    .pop())
}

/// Ends `session` at `now`: removes every lock taken through it in any
/// project and records it as ended, so no acquire still in flight takes
/// another; sessions ended longer ago than any lock lives are forgotten.
/// Returns how many locks went.
///
/// # Errors
/// Returns a [`ConfigError`] when SQLite fails.
pub(super) fn release_all_of(
    conn: &Connection,
    session: &SessionId,
    now: EpochSeconds,
) -> Result<usize, ConfigError> {
    let kept = i64::try_from(LOCK_TTL_MAX.as_secs()).unwrap_or(i64::MAX);
    conn.execute("DELETE FROM ended_sessions WHERE ended_at < ?1", params![
        epoch_param(now).saturating_sub(kept)
    ])
    .map_err(db_error)?;
    conn.execute(
        "INSERT OR REPLACE INTO ended_sessions (session, ended_at) VALUES (?1, ?2)",
        params![session.as_ref(), epoch_param(now)],
    )
    .map_err(db_error)?;
    conn.execute("DELETE FROM locks WHERE session = ?1", params![
        session.as_ref()
    ])
    .map_err(db_error)
}

/// Every lock of project `project_id` live at `now`, by name.
///
/// # Errors
/// Returns a [`ConfigError`] when SQLite fails or a stored row is invalid.
pub(super) fn live(
    conn: &Connection,
    project_id: i64,
    now: EpochSeconds,
) -> Result<Vec<Lock>, ConfigError> {
    collect(
        conn,
        &format!(
            "SELECT {LOCK_COLUMNS} FROM locks WHERE project_id = ?1 AND expires_at >= ?2 \
             ORDER BY name"
        ),
        params![project_id, epoch_param(now)],
        lock_from_row,
    )
}
