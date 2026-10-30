use std::{path::PathBuf, time::UNIX_EPOCH};

use rusqlite::params;

use super::{
    project_rows::find_project,
    sqlite_store::{SqliteStore, db_error, duration_from_millis, duration_millis},
};
use crate::domain::{
    config::ConfigError, occupancy::PendingIdleStop, port::PendingIdleStops, value::ProjectRoot,
};

/// What a pending idle stop's leave time is called in a refusal.
const LEFT_AT: &str = "idle stop leave time";

impl PendingIdleStops for SqliteStore {
    fn pending_idle_stops(&self) -> Result<Vec<PendingIdleStop>, ConfigError> {
        self.read(|conn| {
            let mut statement = conn
                .prepare(
                    "SELECT projects.root, left_at_ms FROM pending_idle_stops \
                     JOIN projects ON projects.id = pending_idle_stops.project_id \
                     ORDER BY projects.root",
                )
                .map_err(db_error)?;
            let mut rows = statement.query([]).map_err(db_error)?;
            let mut stops = Vec::new();
            while let Some(row) = rows.next().map_err(db_error)? {
                let root: String = row.get(0).map_err(db_error)?;
                let left_ms: i64 = row.get(1).map_err(db_error)?;
                let Ok(project) = ProjectRoot::try_new(PathBuf::from(root)) else {
                    continue;
                };
                stops.push(
                    PendingIdleStop::builder()
                        .project(project)
                        .left_at(UNIX_EPOCH + duration_from_millis(LEFT_AT, left_ms)?)
                        .build(),
                );
            }
            Ok(stops)
        })
    }

    fn arm_idle_stop(&self, stop: &PendingIdleStop) -> Result<(), ConfigError> {
        let left_ms = duration_millis(
            LEFT_AT,
            stop.left_at()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default(),
        )?;
        self.write(|conn| {
            let Some((project_id, _)) = find_project(conn, stop.project())? else {
                return Ok(());
            };
            conn.execute(
                "INSERT INTO pending_idle_stops (project_id, left_at_ms) VALUES (?1, ?2) \
                 ON CONFLICT (project_id) DO UPDATE SET left_at_ms = excluded.left_at_ms",
                params![project_id, left_ms],
            )
            .map_err(db_error)?;
            Ok(())
        })
    }

    fn clear_idle_stop(&self, root: &ProjectRoot) -> Result<(), ConfigError> {
        self.write(|conn| {
            let Some((project_id, _)) = find_project(conn, root)? else {
                return Ok(());
            };
            conn.execute(
                "DELETE FROM pending_idle_stops WHERE project_id = ?1",
                params![project_id],
            )
            .map_err(db_error)?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::{
        adapter::store::test_support::{fresh_store, spec},
        domain::{
            config::ImportMode,
            port::{ProjectStore, StackStore},
        },
    };

    /// A moment stored to the millisecond.
    const LEFT_MS: u64 = 1_793_288_081_250;

    /// A pending stop is kept across opens, replaced by a later arming,
    /// forgotten when cleared, and gone with its project.
    #[test]
    fn a_pending_idle_stop_is_kept_replaced_cleared_and_removed_with_its_project() {
        let (store, scratch) = fresh_store();
        let root = ProjectRoot::try_new(scratch.path().join("shop")).unwrap();
        store
            .import(&root, &[spec("api", "true")], ImportMode::Strict)
            .unwrap();
        let stop = |ms: u64| {
            PendingIdleStop::builder()
                .project(root.clone())
                .left_at(UNIX_EPOCH + Duration::from_millis(ms))
                .build()
        };
        assert!(store.pending_idle_stops().unwrap().is_empty());

        store.arm_idle_stop(&stop(LEFT_MS)).unwrap();
        store.arm_idle_stop(&stop(LEFT_MS + 1)).unwrap();
        assert_eq!(store.pending_idle_stops().unwrap(), [stop(LEFT_MS + 1)]);

        store.clear_idle_stop(&root).unwrap();
        assert!(store.pending_idle_stops().unwrap().is_empty());

        store.arm_idle_stop(&stop(LEFT_MS)).unwrap();
        store.remove_project(&root).unwrap();
        assert!(store.pending_idle_stops().unwrap().is_empty());
    }

    /// A project no longer registered is skipped, not refused.
    #[test]
    fn an_unregistered_project_is_skipped() {
        let (store, scratch) = fresh_store();
        let root = ProjectRoot::try_new(scratch.path().join("gone")).unwrap();
        let stop = PendingIdleStop::builder()
            .project(root.clone())
            .left_at(UNIX_EPOCH + Duration::from_millis(LEFT_MS))
            .build();

        store.arm_idle_stop(&stop).unwrap();
        store.clear_idle_stop(&root).unwrap();

        assert!(store.pending_idle_stops().unwrap().is_empty());
    }
}
