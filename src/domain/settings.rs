use getset::{Getters, WithSetters};
use typed_builder::TypedBuilder;

use crate::domain::{
    color_scheme::ColorScheme,
    config::{McpStackEdits, ProcessDefaults, SettingChange, SettingKey},
    process::{RestartPolicy, StopPolicy, StopSignal},
    pty::PtySize,
    value::{DetachKey, LogMaxSize, ScrollbackLines, StopGrace},
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
    log_max_size: LogMaxSize,
    /// Lines of scrollback each process's terminal keeps.
    #[getset(get = "pub", set_with = "pub")]
    scrollback: ScrollbackLines,
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
    default_stop_grace: StopGrace,
    /// Whether an agent over MCP may edit the stack.
    #[getset(get = "pub", set_with = "pub")]
    mcp_stack_edits: McpStackEdits,
}

impl Settings {
    /// These settings with `change` applied.
    pub fn with_change(self, change: &SettingChange) -> Self {
        match change {
            SettingChange::DesktopNotifications(value) => self.with_desktop_notifications(*value),
            SettingChange::DefaultSize(size) => self.with_default_size(*size),
            SettingChange::DetachKey(key) => self.with_detach_key(key.clone()),
            SettingChange::LogMaxSize(size) => self.with_log_max_size(*size),
            SettingChange::Scrollback(lines) => self.with_scrollback(*lines),
            SettingChange::ColorScheme(scheme) => self.with_color_scheme(*scheme),
            SettingChange::DefaultAutostart(value) => self.with_default_autostart(*value),
            SettingChange::DefaultRestart(policy) => self.with_default_restart(*policy),
            SettingChange::DefaultStopSignal(signal) => self.with_default_stop_signal(*signal),
            SettingChange::DefaultStopGrace(grace) => self.with_default_stop_grace(*grace),
            SettingChange::McpStackEdits(edits) => self.with_mcp_stack_edits(*edits),
        }
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
            SettingKey::McpStackEdits => self.mcp_stack_edits.to_string(),
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
    use crate::domain::config::ConfigError;

    /// A PTY of `rows` by `cols`.
    fn size(rows: u16, cols: u16) -> PtySize {
        PtySize::try_new(rows, cols).unwrap()
    }

    /// Settings with every value explicit.
    fn settings() -> Settings {
        Settings::builder()
            .desktop_notifications(true)
            .default_size(size(24, 80))
            .detach_key(DetachKey::try_new("ctrl-\\").unwrap())
            .log_max_size("10MiB".parse().unwrap())
            .scrollback(ScrollbackLines::try_new(1000).unwrap())
            .color_scheme(ColorScheme::Dark)
            .default_autostart(false)
            .default_restart(RestartPolicy::Never)
            .default_stop_signal(StopSignal::Terminate)
            .default_stop_grace(StopGrace::try_new(Duration::from_secs(5)).unwrap())
            .mcp_stack_edits(McpStackEdits::Allow)
            .build()
    }

    #[test]
    fn a_change_sets_only_its_setting() {
        let changed = settings().with_change(&SettingChange::DefaultRestart(RestartPolicy::Always));

        assert_eq!(
            changed,
            settings().with_default_restart(RestartPolicy::Always)
        );
    }

    /// A default size outside the PTY bounds is refused as it is read, so no
    /// setting can hold one.
    #[test]
    fn refuses_a_default_size_outside_the_pty_bounds() {
        for refused in ["0x80", "24x1", "24x2001"] {
            assert!(
                matches!(
                    SettingChange::parse(SettingKey::DefaultSize, refused),
                    Err(ConfigError::InvalidDefaultSize { .. })
                ),
                "{refused}"
            );
        }
        let change = SettingChange::parse(SettingKey::DefaultSize, "30x120").unwrap();
        assert_eq!(
            *settings().with_change(&change).default_size(),
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
            assert_eq!(settings().with_change(&change), settings(), "{key}");
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
