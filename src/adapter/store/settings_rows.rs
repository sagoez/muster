use bytesize::ByteSize as RawByteSize;
use rusqlite::{Connection, Row, params};

use super::{
    edit_log_rows::record_edit_everywhere,
    sqlite_store::{
        SqliteStore, column_integer, corrupt, db_error, duration_millis, grace_from_millis,
    },
    store_error::StoreError,
};
use crate::domain::{
    config::{ConfigError, SettingChange, StackEdit},
    coordination::Author,
    port::{SettingsReader, SettingsStore},
    pty::PtySize,
    settings::Settings,
    value::{ByteSize, DetachKey, LogMaxSize, ScrollbackLines},
};

/// Selects the settings columns [`settings_from_row`] reads by name.
const SELECT_SETTINGS: &str = "SELECT desktop_notifications, default_rows, default_cols, \
                               detach_key, log_max_size_bytes, scrollback_lines, color_scheme, \
                               default_autostart, default_restart, default_stop_signal, \
                               default_stop_grace_ms, mcp_stack_edits FROM settings WHERE id = 1";

/// Reads a [`SELECT_SETTINGS`] row, validating each value through the domain
/// so a hand edit cannot smuggle in a value a write would refuse.
fn settings_from_row(row: &Row) -> Result<Settings, ConfigError> {
    let rows: u16 = row.get("default_rows").map_err(db_error)?;
    let cols: u16 = row.get("default_cols").map_err(db_error)?;
    let detach_key: String = row.get("detach_key").map_err(db_error)?;
    let log_max_size: i64 = row.get("log_max_size_bytes").map_err(db_error)?;
    let scrollback: i64 = row.get("scrollback_lines").map_err(db_error)?;
    let color_scheme: String = row.get("color_scheme").map_err(db_error)?;
    let default_restart: String = row.get("default_restart").map_err(db_error)?;
    let default_stop_signal: String = row.get("default_stop_signal").map_err(db_error)?;
    let default_stop_grace_ms: i64 = row.get("default_stop_grace_ms").map_err(db_error)?;
    let mcp_stack_edits: String = row.get("mcp_stack_edits").map_err(db_error)?;
    let default_size = PtySize::try_new(rows, cols)
        .map_err(|source| ConfigError::InvalidDefaultSize { source })?;
    let log_max_size = u64::try_from(log_max_size)
        .map_err(|error| corrupt("log size", error))
        .and_then(|bytes| {
            ByteSize::try_new(RawByteSize::b(bytes)).map_err(|error| corrupt("log size", error))
        })
        .and_then(|size| LogMaxSize::try_new(size).map_err(|error| corrupt("log size", error)))?;
    let scrollback = usize::try_from(scrollback)
        .map_err(|error| corrupt("scrollback", error))
        .and_then(|lines| {
            ScrollbackLines::try_new(lines).map_err(|error| corrupt("scrollback", error))
        })?;
    Ok(Settings::builder()
        .desktop_notifications(row.get("desktop_notifications").map_err(db_error)?)
        .default_size(default_size)
        .detach_key(DetachKey::try_new(detach_key).map_err(|error| corrupt("detach key", error))?)
        .log_max_size(log_max_size)
        .scrollback(scrollback)
        .color_scheme(
            color_scheme
                .parse()
                .map_err(|error| corrupt("color scheme", error))?,
        )
        .default_autostart(row.get("default_autostart").map_err(db_error)?)
        .default_restart(
            default_restart
                .parse()
                .map_err(|error| corrupt("default restart policy", error))?,
        )
        .default_stop_signal(
            default_stop_signal
                .parse()
                .map_err(|error| corrupt("default stop signal", error))?,
        )
        .default_stop_grace(grace_from_millis(
            "default stop grace",
            default_stop_grace_ms,
        )?)
        .mcp_stack_edits(
            mcp_stack_edits
                .parse()
                .map_err(|error| corrupt("MCP stack edits", error))?,
        )
        .build())
}

/// Reads the one settings row the initial migration seeds.
fn load_settings(conn: &Connection) -> Result<Settings, ConfigError> {
    let mut statement = conn.prepare(SELECT_SETTINGS).map_err(db_error)?;
    let mut rows = statement.query([]).map_err(db_error)?;
    match rows.next().map_err(db_error)? {
        Some(row) => settings_from_row(row),
        None => Err(StoreError::MissingSettings.into()),
    }
}

/// Overwrites the settings row with `settings`.
fn store_settings(conn: &Connection, settings: &Settings) -> Result<(), ConfigError> {
    conn.execute(
        "UPDATE settings SET desktop_notifications = ?1, default_rows = ?2, default_cols = ?3, \
         detach_key = ?4, log_max_size_bytes = ?5, scrollback_lines = ?6, color_scheme = ?7, \
         default_autostart = ?8, default_restart = ?9, default_stop_signal = ?10, \
         default_stop_grace_ms = ?11, mcp_stack_edits = ?12 WHERE id = 1",
        params![
            *settings.desktop_notifications(),
            settings.default_size().rows().into_inner(),
            settings.default_size().cols().into_inner(),
            settings.detach_key().as_ref(),
            column_integer("log size", settings.log_max_size().bytes())?,
            column_integer("scrollback", settings.scrollback().into_inner())?,
            settings.color_scheme().to_string(),
            *settings.default_autostart(),
            settings.default_restart().to_string(),
            settings.default_stop_signal().to_string(),
            duration_millis(
                "default stop grace",
                settings.default_stop_grace().into_inner()
            )?,
            settings.mcp_stack_edits().to_string(),
        ],
    )
    .map_err(db_error)?;
    Ok(())
}

impl SettingsReader for SqliteStore {
    fn load(&self) -> Result<Settings, ConfigError> {
        self.read(load_settings)
    }
}

impl SettingsStore for SqliteStore {
    fn apply(&self, change: &SettingChange, author: &Author) -> Result<Settings, ConfigError> {
        self.write(|conn| {
            let settings = load_settings(conn)?.with_change(change);
            store_settings(conn, &settings)?;
            let stored = load_settings(conn)?;
            let key = change.key();
            record_edit_everywhere(conn, &StackEdit::Configured {
                author: author.clone(),
                key,
                value: stored.value_text(key),
            })?;
            Ok(stored)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::{
        adapter::store::test_support::{database, fresh_store, spec},
        domain::{
            color_scheme::ColorScheme,
            config::{McpStackEdits, SettingKey},
            port::{StackEditLog, StackStore},
            process::{RestartPolicy, StopSignal},
            value::ProjectRoot,
        },
    };

    fn parsed(key: SettingKey, raw: &str) -> SettingChange {
        SettingChange::parse(key, raw).unwrap()
    }

    /// The migration seeds the documented defaults.
    #[test]
    fn a_fresh_database_holds_the_default_settings() {
        let (store, _scratch) = fresh_store();

        let settings = store.load().unwrap();

        assert!(*settings.desktop_notifications());
        assert_eq!(settings.default_size().to_string(), "24x80");
        assert_eq!(settings.detach_key().as_ref(), "ctrl-\\");
        assert_eq!(settings.log_max_size().bytes(), 10 * 1024 * 1024);
        assert_eq!(settings.scrollback().into_inner(), 1000);
        assert_eq!(*settings.color_scheme(), ColorScheme::Dark);
        assert!(!*settings.default_autostart());
        assert_eq!(*settings.default_restart(), RestartPolicy::Never);
        assert_eq!(*settings.default_stop_signal(), StopSignal::Terminate);
        assert_eq!(settings.default_stop_grace().to_string(), "5s");
        assert_eq!(*settings.mcp_stack_edits(), McpStackEdits::Allow);
    }

    /// Every setting written reads back, through another connection too.
    #[test]
    fn every_setting_round_trips() {
        let (store, scratch) = fresh_store();
        let changes = [
            parsed(SettingKey::DesktopNotifications, "false"),
            parsed(SettingKey::DefaultSize, "30x120"),
            parsed(SettingKey::DetachKey, "ctrl-]"),
            parsed(SettingKey::LogMaxSize, "2MiB"),
            parsed(SettingKey::Scrollback, "0"),
            parsed(SettingKey::ColorScheme, "light"),
            parsed(SettingKey::DefaultAutostart, "true"),
            parsed(SettingKey::DefaultRestart, "on-failure"),
            parsed(SettingKey::DefaultStopSignal, "interrupt"),
            parsed(SettingKey::DefaultStopGrace, "1m 30s"),
            parsed(SettingKey::McpStackEdits, "read-only"),
        ];

        let mut expected = store.load().unwrap();
        for change in &changes {
            expected = expected.with_change(change);
            assert_eq!(
                store.apply(change, &Author::human()).unwrap(),
                expected,
                "{}",
                change.key()
            );
        }

        assert_eq!(store.load().unwrap(), expected);
        let reopened = SqliteStore::open(&database(&scratch)).unwrap();
        assert_eq!(reopened.load().unwrap(), expected);
    }

    /// A default size edited outside the PTY bounds by hand is refused as
    /// it is read, so no settings can hold one.
    #[test]
    fn a_hand_edited_size_outside_the_bounds_is_refused() {
        let (store, scratch) = fresh_store();
        Connection::open(database(&scratch))
            .unwrap()
            .execute("UPDATE settings SET default_rows = 0", [])
            .unwrap();

        let error = store.load().unwrap_err();

        assert!(matches!(error, ConfigError::InvalidDefaultSize { .. }));
    }

    /// A hand-edited row the domain refuses fails to load rather than
    /// reaching a launch.
    #[test]
    fn a_hand_edited_invalid_row_is_refused() {
        let (store, _scratch) = fresh_store();
        store
            .write(|conn| {
                conn.execute("UPDATE settings SET detach_key = 'alt-x'", [])
                    .map_err(db_error)
            })
            .unwrap();

        assert!(matches!(store.load(), Err(ConfigError::CorruptRow { .. })));
    }

    /// A hand-edited value past a setting's bound is refused on read, as a
    /// write would refuse it.
    #[test]
    fn a_hand_edited_value_out_of_bounds_is_refused() {
        for edit in [
            "scrollback_lines = 100001",
            "log_max_size_bytes = 1024",
            "log_max_size_bytes = 2147483648",
            "default_stop_grace_ms = 3600001",
        ] {
            let (store, _scratch) = fresh_store();
            store
                .write(|conn| {
                    conn.execute(&format!("UPDATE settings SET {edit}"), [])
                        .map_err(db_error)
                })
                .unwrap();

            assert!(
                matches!(store.load(), Err(ConfigError::CorruptRow { .. })),
                "{edit}"
            );
        }
    }

    /// A change answers with the value the database holds, which rounds a
    /// grace up to whole milliseconds.
    #[test]
    fn a_change_answers_with_the_stored_value() {
        let (store, _scratch) = fresh_store();

        let settings = store
            .apply(
                &parsed(SettingKey::DefaultStopGrace, "1500us"),
                &Author::human(),
            )
            .unwrap();

        assert_eq!(settings.value_text(SettingKey::DefaultStopGrace), "2ms");
        assert_eq!(settings, store.load().unwrap());
    }

    /// An agent's change is kept, with the value stored, in every project's
    /// edit log under no process; the human's is not.
    #[test]
    fn an_agents_change_is_logged_in_every_project() {
        let (store, scratch) = fresh_store();
        let roots: Vec<ProjectRoot> = ["shop", "blog"]
            .iter()
            .map(|folder| {
                let dir = scratch.path().join(folder);
                fs::create_dir_all(&dir).unwrap();
                let root = ProjectRoot::try_new(dir.canonicalize().unwrap()).unwrap();
                store
                    .add_process(&root, &spec("web", "true"), &Author::human())
                    .unwrap();
                root
            })
            .collect();

        store
            .apply(
                &parsed(SettingKey::DefaultRestart, "always"),
                &Author::human(),
            )
            .unwrap();
        store
            .apply(
                &parsed(SettingKey::DefaultStopGrace, "1500us"),
                &Author::agent("claude"),
            )
            .unwrap();

        for root in &roots {
            let edits = store.edits(root).unwrap();
            assert_eq!(edits.len(), 1, "{edits:?}");
            assert_eq!(edits[0].process(), &None);
            assert_eq!(
                edits[0].message(),
                "agent claude set default-stop-grace to 2ms"
            );
        }
    }
}
