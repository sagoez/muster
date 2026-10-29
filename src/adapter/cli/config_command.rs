//! `muster config`: list, read and change the settings.

use serde_json::{Map, Value, json};
use strum::IntoEnumIterator;
use unicode_width::UnicodeWidthStr;

use super::{
    error::CliError,
    invocation::{Invocation, stack_editor},
    report::{Report, Row, RowKind},
    setting_view::SettingView,
};
use crate::domain::{
    config::{SettingChange, SettingKey},
    port::SettingsStore,
    settings::Settings,
};

/// Title of the `config set` report.
const CONFIG_TITLE: &str = "muster config";
/// Between a key and its value in the list and in the `set` line.
const COLUMN_GAP: &str = "  ";
/// Between a key and its new value in the `set` line.
const ASSIGNS: &str = " = ";
/// Separates the words of a kebab-case key, replaced for a JSON key.
const KEBAB: char = '-';
/// Separates the words of a JSON key.
const SNAKE: &str = "_";

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

/// Every setting as one JSON object: snake_case keys, and each value typed,
/// a boolean, a number of lines, bytes or seconds, or the text
/// `config get` prints.
pub fn settings_json(settings: &Settings) -> Value {
    Value::Object(
        SettingKey::iter()
            .map(|key| {
                (
                    key.to_string().replace(KEBAB, SNAKE),
                    typed_value(settings, key),
                )
            })
            .collect::<Map<String, Value>>(),
    )
}

/// The value of `key` in `settings`, typed for JSON.
fn typed_value(settings: &Settings, key: SettingKey) -> Value {
    match key {
        SettingKey::DesktopNotifications => json!(settings.desktop_notifications()),
        SettingKey::DefaultAutostart => json!(settings.default_autostart()),
        SettingKey::Scrollback => json!(settings.scrollback()),
        SettingKey::LogMaxSize => json!(settings.log_max_size().bytes()),
        SettingKey::DefaultStopGrace => {
            json!(settings.default_stop_grace().into_inner().as_secs_f64())
        },
        SettingKey::DefaultSize
        | SettingKey::DetachKey
        | SettingKey::ColorScheme
        | SettingKey::DefaultRestart
        | SettingKey::DefaultStopSignal => json!(settings.value_text(key)),
    }
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
    use crate::domain::{
        color_scheme::ColorScheme,
        process::{RestartPolicy, StopSignal},
        pty::PtySize,
        value::{ByteSize, DetachKey, PositiveDuration},
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

    /// The JSON keys are snake_case and the values typed.
    #[test]
    fn the_json_is_typed() {
        let value = settings_json(&seeded());

        assert_eq!(value["desktop_notifications"], true);
        assert_eq!(value["scrollback"], 1000);
        assert_eq!(value["log_max_size"], 10 * 1024 * 1024);
        assert_eq!(value["default_stop_grace"], 5.0);
        assert_eq!(value["default_size"], "24x80");
        assert_eq!(value["detach_key"], "ctrl-\\");
        assert_eq!(value.as_object().unwrap().len(), SettingKey::iter().count());
    }
}
