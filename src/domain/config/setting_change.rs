use crate::domain::{
    color_scheme::ColorScheme,
    config::{ConfigError, SettingKey},
    process::{RestartPolicy, StopSignal},
    pty::PtySize,
    value::{ByteSize, DetachKey, PositiveDuration, parse_as, parse_choice},
};

/// The form a boolean setting is written in.
const BOOLEAN_FORM: &str = "true or false";
/// The form a line count is written in.
const LINE_COUNT_FORM: &str = "a whole number of lines such as 1000";
/// The form a detach key is written in.
const DETACH_KEY_FORM: &str = "a Control chord such as ctrl-\\ or ctrl-a";

/// A new value for one setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingChange {
    /// Whether to raise OS desktop notifications.
    DesktopNotifications(bool),
    /// Terminal size a process starts with.
    DefaultSize(PtySize),
    /// The key that detaches `muster attach`.
    DetachKey(DetachKey),
    /// Size at which a process's output log rotates.
    LogMaxSize(ByteSize),
    /// Lines of scrollback each process's terminal keeps.
    Scrollback(usize),
    /// The default colors a process's terminal reports.
    ColorScheme(ColorScheme),
    /// Whether a new process starts with `muster up`.
    DefaultAutostart(bool),
    /// A new process's restart policy.
    DefaultRestart(RestartPolicy),
    /// A new process's graceful stop signal.
    DefaultStopSignal(StopSignal),
    /// A new process's grace period before force-kill.
    DefaultStopGrace(PositiveDuration),
}

impl SettingChange {
    /// Parses `raw` as the value of `key`.
    ///
    /// # Errors
    /// Returns [`ConfigError::InvalidSetting`] naming the key and the form it
    /// accepts.
    pub fn parse(key: SettingKey, raw: &str) -> Result<Self, ConfigError> {
        let raw = raw.trim();
        let change = match key {
            SettingKey::DesktopNotifications => {
                parse_as(raw, BOOLEAN_FORM).map(Self::DesktopNotifications)
            },
            SettingKey::DefaultSize => raw.parse().map(Self::DefaultSize),
            SettingKey::DetachKey => parse_as(raw, DETACH_KEY_FORM).map(Self::DetachKey),
            SettingKey::LogMaxSize => raw.parse().map(Self::LogMaxSize),
            SettingKey::Scrollback => parse_as(raw, LINE_COUNT_FORM).map(Self::Scrollback),
            SettingKey::ColorScheme => parse_choice(raw).map(Self::ColorScheme),
            SettingKey::DefaultAutostart => parse_as(raw, BOOLEAN_FORM).map(Self::DefaultAutostart),
            SettingKey::DefaultRestart => parse_choice(raw).map(Self::DefaultRestart),
            SettingKey::DefaultStopSignal => parse_choice(raw).map(Self::DefaultStopSignal),
            SettingKey::DefaultStopGrace => raw.parse().map(Self::DefaultStopGrace),
        };
        change.map_err(|error| ConfigError::InvalidSetting {
            key,
            reason: error.to_string(),
        })
    }

    /// The setting this change sets.
    pub fn key(&self) -> SettingKey {
        match self {
            Self::DesktopNotifications(_) => SettingKey::DesktopNotifications,
            Self::DefaultSize(_) => SettingKey::DefaultSize,
            Self::DetachKey(_) => SettingKey::DetachKey,
            Self::LogMaxSize(_) => SettingKey::LogMaxSize,
            Self::Scrollback(_) => SettingKey::Scrollback,
            Self::ColorScheme(_) => SettingKey::ColorScheme,
            Self::DefaultAutostart(_) => SettingKey::DefaultAutostart,
            Self::DefaultRestart(_) => SettingKey::DefaultRestart,
            Self::DefaultStopSignal(_) => SettingKey::DefaultStopSignal,
            Self::DefaultStopGrace(_) => SettingKey::DefaultStopGrace,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::domain::value::{Cols, Rows};

    fn parse(key: SettingKey, raw: &str) -> SettingChange {
        SettingChange::parse(key, raw).unwrap()
    }

    fn refusal(key: SettingKey, raw: &str) -> String {
        let error = SettingChange::parse(key, raw).unwrap_err();
        assert!(
            matches!(error, ConfigError::InvalidSetting { key: refused, .. } if refused == key),
            "{error}"
        );
        error.to_string()
    }

    #[test]
    fn booleans_accept_exactly_true_and_false() {
        assert_eq!(
            parse(SettingKey::DesktopNotifications, "false"),
            SettingChange::DesktopNotifications(false)
        );
        assert_eq!(
            parse(SettingKey::DefaultAutostart, "true"),
            SettingChange::DefaultAutostart(true)
        );
        for refused in ["yes", "1", "True"] {
            assert!(refusal(SettingKey::DefaultAutostart, refused).contains(BOOLEAN_FORM));
        }
    }

    #[test]
    fn default_size_reads_rows_by_columns() {
        assert_eq!(
            parse(SettingKey::DefaultSize, "30x120"),
            SettingChange::DefaultSize(
                PtySize::builder()
                    .rows(Rows::new(30))
                    .cols(Cols::new(120))
                    .build()
            )
        );
        assert!(refusal(SettingKey::DefaultSize, "big").contains("ROWSxCOLS"));
    }

    #[test]
    fn detach_key_reads_a_control_chord() {
        assert_eq!(
            parse(SettingKey::DetachKey, "ctrl-]"),
            SettingChange::DetachKey(DetachKey::try_new("ctrl-]").unwrap())
        );
        assert!(refusal(SettingKey::DetachKey, "alt-a").contains("Control chord"));
    }

    #[test]
    fn log_max_size_reads_a_positive_size() {
        assert_eq!(
            parse(SettingKey::LogMaxSize, "20MiB"),
            SettingChange::LogMaxSize("20MiB".parse().unwrap())
        );
        refusal(SettingKey::LogMaxSize, "0B");
    }

    #[test]
    fn scrollback_reads_a_line_count() {
        assert_eq!(
            parse(SettingKey::Scrollback, "0"),
            SettingChange::Scrollback(0)
        );
        assert!(refusal(SettingKey::Scrollback, "-1").contains(LINE_COUNT_FORM));
    }

    #[test]
    fn enumerated_settings_list_their_choices() {
        assert_eq!(
            parse(SettingKey::ColorScheme, "light"),
            SettingChange::ColorScheme(ColorScheme::Light)
        );
        assert_eq!(
            parse(SettingKey::DefaultRestart, "on-failure"),
            SettingChange::DefaultRestart(RestartPolicy::OnFailure)
        );
        assert_eq!(
            parse(SettingKey::DefaultStopSignal, "interrupt"),
            SettingChange::DefaultStopSignal(StopSignal::Interrupt)
        );
        assert!(refusal(SettingKey::ColorScheme, "blue").contains("dark, light"));
        assert!(
            refusal(SettingKey::DefaultRestart, "sometimes").contains("never, on-failure, always")
        );
        assert!(refusal(SettingKey::DefaultStopSignal, "kill").contains("terminate, interrupt"));
    }

    #[test]
    fn default_stop_grace_reads_a_positive_duration() {
        assert_eq!(
            parse(SettingKey::DefaultStopGrace, "10s"),
            SettingChange::DefaultStopGrace(
                PositiveDuration::try_new(Duration::from_secs(10)).unwrap()
            )
        );
        refusal(SettingKey::DefaultStopGrace, "0s");
    }

    /// The error names the key so a caller can tell which setting refused.
    #[test]
    fn a_refusal_names_the_key() {
        assert!(refusal(SettingKey::Scrollback, "many").contains("scrollback"));
    }
}
