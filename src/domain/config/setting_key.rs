use strum::{Display, EnumIter, EnumString};

/// Names one user setting, as `muster config` and MCP address it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumIter, EnumString, Display)]
#[strum(serialize_all = "kebab-case")]
pub enum SettingKey {
    /// Whether to raise OS desktop notifications.
    DesktopNotifications,
    /// Terminal size a process starts with.
    DefaultSize,
    /// The key that detaches `muster attach`.
    DetachKey,
    /// Size at which a process's output log rotates.
    LogMaxSize,
    /// Lines of scrollback each process's terminal keeps.
    Scrollback,
    /// The default colors a process's terminal reports.
    ColorScheme,
    /// Whether a new process starts with `muster up`.
    DefaultAutostart,
    /// A new process's restart policy.
    DefaultRestart,
    /// A new process's graceful stop signal.
    DefaultStopSignal,
    /// A new process's grace period before force-kill.
    DefaultStopGrace,
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;

    /// Every key reads back from the kebab-case name it displays as.
    #[test]
    fn keys_round_trip_their_kebab_case_names() {
        for key in SettingKey::iter() {
            assert_eq!(key.to_string().parse::<SettingKey>().unwrap(), key);
        }
        assert_eq!(
            SettingKey::DefaultStopGrace.to_string(),
            "default-stop-grace"
        );
    }
}
