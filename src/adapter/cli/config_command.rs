//! `muster config`: list, read and change the settings.

use strum::IntoEnumIterator;
use unicode_width::UnicodeWidthStr;

use super::{
    error::CliError,
    invocation::{Invocation, stack_editor},
    report::{Report, Row, RowKind},
};
use crate::{
    adapter::client::SettingView,
    domain::{
        config::{SettingChange, SettingKey},
        port::SettingsStore,
        settings::Settings,
    },
};

/// Title of the `config set` report.
const CONFIG_TITLE: &str = "muster config";
/// Between a key and its value in the list and in the `set` line.
const COLUMN_GAP: &str = "  ";
/// Between a key and its new value in the `set` line.
const ASSIGNS: &str = " = ";

/// Every setting.
///
/// # Errors
/// Returns the [`CliError`] of settings that cannot be read.
pub fn settings(invocation: &Invocation<'_>) -> Result<Settings, CliError> {
    Ok(invocation.store()?.load()?)
}

/// Changes the setting `key` to `raw`, read as `config get` prints it, and
/// tells a running service.
///
/// # Errors
/// Returns the [`CliError::Config`] of a value the setting does not accept,
/// or of settings that cannot be written.
pub fn set_setting(
    invocation: &Invocation<'_>,
    key: SettingKey,
    raw: &str,
) -> Result<SettingView, CliError> {
    let change = SettingChange::parse(key, raw)?;
    let store = invocation.store()?;
    let settings = stack_editor(&store).configure(&change)?;
    Ok(SettingView::of(&settings, key))
}

/// The `config list` table: one `key  value` line per setting, aligned.
pub fn settings_table(settings: &Settings) -> String {
    let width = SettingKey::iter()
        .map(|key| UnicodeWidthStr::width(key.to_string().as_str()))
        .max()
        .unwrap_or(0);
    SettingKey::iter()
        .map(|key| {
            format!(
                "{:width$}{COLUMN_GAP}{}",
                key.to_string(),
                settings.value_text(key)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The framed line `config set` prints: `detach-key = ctrl-]`.
pub fn set_rows(view: &SettingView) -> Report {
    Report::new(CONFIG_TITLE, vec![Row::unlabeled(
        RowKind::Ok,
        format!("{}{ASSIGNS}{}", view.key(), view.value()),
    )])
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::{
        adapter::client::SettingsView,
        domain::{
            color_scheme::ColorScheme,
            process::{RestartPolicy, StopSignal},
            pty::PtySize,
            value::{ByteSize, DetachKey, PositiveDuration},
        },
    };

    /// Settings as the initial migration seeds them.
    fn seeded() -> Settings {
        Settings::builder()
            .desktop_notifications(true)
            .default_size("24x80".parse::<PtySize>().unwrap())
            .detach_key(DetachKey::try_new("ctrl-\\").unwrap())
            .log_max_size("10MiB".parse::<ByteSize>().unwrap())
            .scrollback(1000)
            .color_scheme(ColorScheme::Dark)
            .default_autostart(false)
            .default_restart(RestartPolicy::Never)
            .default_stop_signal(StopSignal::Terminate)
            .default_stop_grace(PositiveDuration::try_new(Duration::from_secs(5)).unwrap())
            .build()
    }

    /// The list is one aligned line per setting, each value in the form
    /// `config set` reads back.
    #[test]
    fn the_list_has_one_aligned_line_per_setting() {
        let table = settings_table(&seeded());

        assert_eq!(table.lines().count(), SettingKey::iter().count());
        assert!(table.contains("detach-key             ctrl-\\"), "{table}");
        assert!(table.contains("default-stop-grace     5s"), "{table}");
    }

    /// The JSON lists every setting under snake_case object keys, each by
    /// its kebab-case key with the value `config set` reads back.
    #[test]
    fn the_json_lists_each_setting_by_key() {
        let value = serde_json::to_value(SettingsView::from(&seeded())).unwrap();
        let settings = value["settings"].as_array().unwrap();

        assert_eq!(settings.len(), SettingKey::iter().count());
        assert!(settings.contains(&serde_json::json!({"key": "detach-key", "value": "ctrl-\\"})));
        assert!(
            settings.contains(&serde_json::json!({"key": "default-stop-grace", "value": "5s"}))
        );
    }
}
