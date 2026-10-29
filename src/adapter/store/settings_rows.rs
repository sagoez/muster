use bytesize::ByteSize as RawByteSize;
use rusqlite::{Connection, Row, params};

use super::sqlite_store::{
    SqliteStore, column_integer, corrupt, db_error, duration_millis, grace_from_millis,
};
use crate::domain::{
    config::{ConfigError, SettingChange},
    port::SettingsStore,
    pty::PtySize,
    settings::Settings,
    value::{ByteSize, Cols, DetachKey, LogMaxSize, Rows, ScrollbackLines},
};

/// Selects the settings row in the order [`settings_from_row`] reads it.
const SELECT_SETTINGS: &str = "SELECT desktop_notifications, default_rows, default_cols, \
                               detach_key, log_max_size_bytes, scrollback_lines, color_scheme, \
                               default_autostart, default_restart, default_stop_signal, \
                               default_stop_grace_ms FROM settings WHERE id = 1";

/// Reads a [`SELECT_SETTINGS`] row, validating each value through the domain
/// so a hand edit cannot smuggle in a value a write would refuse.
fn settings_from_row(row: &Row) -> Result<Settings, ConfigError> {
    let rows: u16 = row.get(1).map_err(db_error)?;
    let cols: u16 = row.get(2).map_err(db_error)?;
    let detach_key: String = row.get(3).map_err(db_error)?;
    let log_max_size: i64 = row.get(4).map_err(db_error)?;
    let scrollback: i64 = row.get(5).map_err(db_error)?;
    let color_scheme: String = row.get(6).map_err(db_error)?;
    let default_restart: String = row.get(8).map_err(db_error)?;
    let default_stop_signal: String = row.get(9).map_err(db_error)?;
    let default_stop_grace_ms: i64 = row.get(10).map_err(db_error)?;
    let default_size = PtySize::builder()
        .rows(Rows::new(rows))
        .cols(Cols::new(cols))
        .build()
        .checked()
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
        .desktop_notifications(row.get(0).map_err(db_error)?)
        .default_size(default_size)
        .detach_key(DetachKey::try_new(detach_key).map_err(|error| corrupt("detach key", error))?)
        .log_max_size(log_max_size)
        .scrollback(scrollback)
        .color_scheme(
            color_scheme
                .parse()
                .map_err(|error| corrupt("color scheme", error))?,
        )
        .default_autostart(row.get(7).map_err(db_error)?)
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
        .build())
}

/// Reads the one settings row the initial migration seeds.
fn load_settings(conn: &Connection) -> Result<Settings, ConfigError> {
    let mut statement = conn.prepare(SELECT_SETTINGS).map_err(db_error)?;
    let mut rows = statement.query([]).map_err(db_error)?;
    match rows.next().map_err(db_error)? {
        Some(row) => settings_from_row(row),
        None => Err(ConfigError::Database("the settings row is missing".into())),
    }
}

/// Overwrites the settings row with `settings`.
fn store_settings(conn: &Connection, settings: &Settings) -> Result<(), ConfigError> {
    conn.execute(
        "UPDATE settings SET desktop_notifications = ?1, default_rows = ?2, default_cols = ?3, \
         detach_key = ?4, log_max_size_bytes = ?5, scrollback_lines = ?6, color_scheme = ?7, \
         default_autostart = ?8, default_restart = ?9, default_stop_signal = ?10, \
         default_stop_grace_ms = ?11 WHERE id = 1",
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
        ],
    )
    .map_err(db_error)?;
    Ok(())
}

impl SettingsStore for SqliteStore {
    fn load(&self) -> Result<Settings, ConfigError> {
        self.read(load_settings)
    }

    fn apply(&self, change: &SettingChange) -> Result<Settings, ConfigError> {
        self.write(|conn| {
            let settings = load_settings(conn)?.with_change(change)?;
            store_settings(conn, &settings)?;
            load_settings(conn)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        adapter::store::test_support::{database, fresh_store},
        domain::{
            color_scheme::ColorScheme,
            config::SettingKey,
            process::{RestartPolicy, StopSignal},
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
        ];

        let mut expected = store.load().unwrap();
        for change in &changes {
            expected = expected.with_change(change).unwrap();
            assert_eq!(store.apply(change).unwrap(), expected, "{}", change.key());
        }

        assert_eq!(store.load().unwrap(), expected);
        let reopened = SqliteStore::open(&database(&scratch)).unwrap();
        assert_eq!(reopened.load().unwrap(), expected);
    }

    /// A refused change writes nothing.
    #[test]
    fn a_refused_change_leaves_the_settings() {
        let (store, _scratch) = fresh_store();
        let before = store.load().unwrap();

        let error = store
            .apply(&parsed(SettingKey::DefaultSize, "0x80"))
            .unwrap_err();

        assert!(matches!(error, ConfigError::InvalidDefaultSize { .. }));
        assert_eq!(store.load().unwrap(), before);
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
            .apply(&parsed(SettingKey::DefaultStopGrace, "1500us"))
            .unwrap();

        assert_eq!(settings.value_text(SettingKey::DefaultStopGrace), "2ms");
        assert_eq!(settings, store.load().unwrap());
    }
}
