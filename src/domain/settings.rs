use getset::{Getters, WithSetters};
use typed_builder::TypedBuilder;

use crate::domain::{
    color_scheme::ColorScheme,
    config::{ConfigError, ProcessDefaults, SettingChange, SettingKey},
    process::{RestartPolicy, StopPolicy, StopSignal},
    pty::PtySize,
    value::{ByteSize, DetachKey, PositiveDuration},
};

/// User settings that apply across every project, stored per machine.
#[derive(Clone, Debug, PartialEq, Eq, Getters, WithSetters, TypedBuilder)]
#[set_with]
pub struct Settings {
    /// Whether to raise OS desktop notifications.
    #[getset(get = "pub", set_with = "pub")]
    desktop_notifications: bool,
    /// Terminal size a process starts with, before any client resizes it.
    #[getset(get = "pub", set_with = "pub")]
    default_size: PtySize,
    /// The key that detaches `muster attach` from a process.
    #[getset(get = "pub", set_with = "pub")]
    detach_key: DetachKey,
    /// Size at which a process's output log rotates.
    #[getset(get = "pub", set_with = "pub")]
    log_max_size: ByteSize,
    /// Lines of scrollback each process's terminal keeps.
    #[getset(get = "pub", set_with = "pub")]
    scrollback: usize,
    /// The default colors a process's terminal reports when asked.
    #[getset(get = "pub", set_with = "pub")]
    color_scheme: ColorScheme,
    /// Whether a newly added process starts with `muster up`.
    #[getset(get = "pub", set_with = "pub")]
    default_autostart: bool,
    /// A newly added process's restart policy.
    #[getset(get = "pub", set_with = "pub")]
    default_restart: RestartPolicy,
    /// A newly added process's graceful stop signal.
    #[getset(get = "pub", set_with = "pub")]
    default_stop_signal: StopSignal,
    /// A newly added process's grace period before force-kill.
    #[getset(get = "pub", set_with = "pub")]
    default_stop_grace: PositiveDuration,
}

impl Settings {
    /// These settings with `change` applied.
    ///
    /// # Errors
    /// Returns [`ConfigError::InvalidDefaultSize`] for a size outside the PTY
    /// bounds.
    pub fn with_change(self, change: &SettingChange) -> Result<Self, ConfigError> {
        Ok(match change {
            SettingChange::DesktopNotifications(value) => self.with_desktop_notifications(*value),
            SettingChange::DefaultSize(size) => self.with_default_size(
                size.checked()
                    .map_err(|source| ConfigError::InvalidDefaultSize { source })?,
            ),
            SettingChange::DetachKey(key) => self.with_detach_key(key.clone()),
            SettingChange::LogMaxSize(size) => self.with_log_max_size(*size),
            SettingChange::Scrollback(lines) => self.with_scrollback(*lines),
            SettingChange::ColorScheme(scheme) => self.with_color_scheme(*scheme),
            SettingChange::DefaultAutostart(value) => self.with_default_autostart(*value),
            SettingChange::DefaultRestart(policy) => self.with_default_restart(*policy),
            SettingChange::DefaultStopSignal(signal) => self.with_default_stop_signal(*signal),
            SettingChange::DefaultStopGrace(grace) => self.with_default_stop_grace(*grace),
        })
    }

    /// The value of `key` as `muster config get` prints it, in the form
    /// [`SettingChange::parse`] reads back.
    pub fn value_text(&self, key: SettingKey) -> String {
        match key {
            SettingKey::DesktopNotifications => self.desktop_notifications.to_string(),
            SettingKey::DefaultSize => self.default_size.to_string(),
            SettingKey::DetachKey => self.detach_key.to_string(),
            SettingKey::LogMaxSize => self.log_max_size.to_string(),
            SettingKey::Scrollback => self.scrollback.to_string(),
            SettingKey::ColorScheme => self.color_scheme.to_string(),
            SettingKey::DefaultAutostart => self.default_autostart.to_string(),
            SettingKey::DefaultRestart => self.default_restart.to_string(),
            SettingKey::DefaultStopSignal => self.default_stop_signal.to_string(),
            SettingKey::DefaultStopGrace => self.default_stop_grace.to_string(),
        }
    }

    /// What a newly added process takes for the fields it leaves out.
    pub fn process_defaults(&self) -> ProcessDefaults {
        ProcessDefaults::builder()
            .autostart(self.default_autostart)
            .restart(self.default_restart)
            .stop(
                StopPolicy::builder()
                    .signal(self.default_stop_signal)
                    .grace_period(self.default_stop_grace)
                    .build(),
            )
            .build()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use strum::IntoEnumIterator;

    use super::*;
    use crate::domain::value::{Cols, Rows};

    fn size(rows: u16, cols: u16) -> PtySize {
        PtySize::builder()
            .rows(Rows::new(rows))
            .cols(Cols::new(cols))
            .build()
    }

    fn settings() -> Settings {
        Settings::builder()
            .desktop_notifications(true)
            .default_size(size(24, 80))
            .detach_key(DetachKey::try_new("ctrl-\\").unwrap())
            .log_max_size("10MiB".parse().unwrap())
            .scrollback(1000)
            .color_scheme(ColorScheme::Dark)
            .default_autostart(false)
            .default_restart(RestartPolicy::Never)
            .default_stop_signal(StopSignal::Terminate)
            .default_stop_grace(PositiveDuration::try_new(Duration::from_secs(5)).unwrap())
            .build()
    }

    #[test]
    fn a_change_sets_only_its_setting() {
        let changed = settings()
            .with_change(&SettingChange::DefaultRestart(RestartPolicy::Always))
            .unwrap();

        assert_eq!(
            changed,
            settings().with_default_restart(RestartPolicy::Always)
        );
    }

    #[test]
    fn refuses_a_default_size_outside_the_pty_bounds() {
        for refused in [size(0, 80), size(24, 1), size(24, PtySize::MAX_COLS + 1)] {
            assert!(matches!(
                settings().with_change(&SettingChange::DefaultSize(refused)),
                Err(ConfigError::InvalidDefaultSize { .. })
            ));
        }
        assert_eq!(
            *settings()
                .with_change(&SettingChange::DefaultSize(size(30, 120)))
                .unwrap()
                .default_size(),
            size(30, 120)
        );
    }

    /// Every value prints in the form `config set` reads, so get-then-set is
    /// a no-op.
    #[test]
    fn every_value_text_parses_back_to_the_same_settings() {
        for key in SettingKey::iter() {
            let change = SettingChange::parse(key, &settings().value_text(key)).unwrap();

            assert_eq!(change.key(), key);
            assert_eq!(
                settings().with_change(&change).unwrap(),
                settings(),
                "{key}"
            );
        }
        assert_eq!(settings().value_text(SettingKey::DefaultSize), "24x80");
        assert_eq!(settings().value_text(SettingKey::DefaultStopGrace), "5s");
    }

    #[test]
    fn process_defaults_come_from_the_default_settings() {
        let defaults = settings()
            .with_default_autostart(true)
            .with_default_stop_signal(StopSignal::Interrupt)
            .process_defaults();

        assert!(*defaults.autostart());
        assert_eq!(*defaults.restart(), RestartPolicy::Never);
        assert_eq!(*defaults.stop().signal(), StopSignal::Interrupt);
        assert_eq!(
            *defaults.stop().grace_period(),
            *settings().default_stop_grace()
        );
    }
}
