use rusqlite::{Connection, OptionalExtension, Row, named_params, params};

use super::{
    coordination_rows::collect,
    sqlite_store::{corrupt, db_error},
    store_error::StoreError,
};
use crate::domain::{
    config::ConfigError,
    coordination::{Author, Lock, LockAttempt, LockName, LockNote, LockRelease, LockTtl},
};

/// The columns [`lock_from_row`] reads, in order.
const LOCK_COLUMNS: &str = "name, owner, note, expires_at";

/// Takes the lock for `me` when it is free, expired or already `me`'s, in one
/// upsert whose `RETURNING` yields no row when someone else holds it.
const ACQUIRE: &str = "\
INSERT INTO locks (project_id, name, owner, note, expires_at) \
VALUES (:project, :name, :me, :note, :until) \
ON CONFLICT (project_id, name) DO UPDATE SET \
    owner = excluded.owner, note = excluded.note, expires_at = excluded.expires_at \
WHERE locks.expires_at < :now OR locks.owner = excluded.owner \
RETURNING expires_at";

/// Reads a lock row selected as [`LOCK_COLUMNS`].
fn lock_from_row(row: &Row) -> Result<Lock, ConfigError> {
    let name: String = row.get(0).map_err(db_error)?;
    let owner: String = row.get(1).map_err(db_error)?;
    let note: Option<String> = row.get(2).map_err(db_error)?;
    let expires_at: i64 = row.get(3).map_err(db_error)?;
    Ok(Lock::builder()
        .name(LockName::try_new(name).map_err(|error| corrupt("lock name", error))?)
        .owner(Author::from_label(&owner))
        .note(
            note.map(LockNote::try_new)
                .transpose()
                .map_err(|error| corrupt("lock note", error))?,
        )
        .expires_at(u64::try_from(expires_at).map_err(|error| corrupt("lock expiry", error))?)
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

/// Takes the lock `name` of project `project_id` for `me` for `ttl` at `now`.
///
/// # Errors
/// Returns a [`ConfigError`] when SQLite fails or a stored row is invalid.
pub(super) fn acquire(
    conn: &Connection,
    project_id: i64,
    me: &Author,
    name: &LockName,
    ttl: LockTtl,
    note: Option<&LockNote>,
    now: u64,
) -> Result<LockAttempt, ConfigError> {
    let until = now + ttl.as_ref().as_secs();
    let taken: Option<i64> = conn
        .query_row(
            ACQUIRE,
            named_params! {
                ":project": project_id,
                ":name": name.as_ref(),
                ":me": me.label(),
                ":note": note.map(AsRef::<str>::as_ref),
                ":until": until as i64,
                ":now": now as i64,
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

/// Releases the lock `name` of project `project_id` when `me` holds it.
///
/// # Errors
/// Returns a [`ConfigError`] when SQLite fails or a stored row is invalid.
pub(super) fn release(
    conn: &Connection,
    project_id: i64,
    me: &Author,
    name: &LockName,
    now: u64,
) -> Result<LockRelease, ConfigError> {
    let removed = conn
        .execute(
            "DELETE FROM locks WHERE project_id = ?1 AND name = ?2 AND owner = ?3",
            params![project_id, name.as_ref(), me.label()],
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

/// Removes every lock `me` holds in any project. Returns how many went.
///
/// # Errors
/// Returns a [`ConfigError`] when SQLite fails.
pub(super) fn release_all_of(conn: &Connection, me: &Author) -> Result<usize, ConfigError> {
    conn.execute("DELETE FROM locks WHERE owner = ?1", params![me.label()])
        .map_err(db_error)
}

/// Every lock of project `project_id` live at `now`, by name.
///
/// # Errors
/// Returns a [`ConfigError`] when SQLite fails or a stored row is invalid.
pub(super) fn live(conn: &Connection, project_id: i64, now: u64) -> Result<Vec<Lock>, ConfigError> {
    collect(
        conn,
        &format!(
            "SELECT {LOCK_COLUMNS} FROM locks WHERE project_id = ?1 AND expires_at >= ?2 \
             ORDER BY name"
        ),
        params![project_id, now as i64],
        lock_from_row,
    )
}
