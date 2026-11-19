use strum::{Display, EnumIter, EnumProperty, EnumString};

/// The property marking a setting an agent over MCP may change.
const AGENT_MAY_SET: &str = "agent_may_set";

/// Names one user setting, as `muster config` and MCP address it. Only the
/// defaults a new process takes are marked `agent_may_set`, so an agent can
/// never lift its own guardrails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumIter, EnumString, Display, EnumProperty)]
#[strum(serialize_all = "kebab-case")]
pub enum SettingKey {
    /// Whether to raise OS desktop notifications.
    DesktopNotifications,
    /// Terminal size a process starts with.
    #[strum(props(agent_may_set = true))]
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
    #[strum(props(agent_may_set = true))]
    DefaultAutostart,
    /// A new process's restart policy.
    #[strum(props(agent_may_set = true))]
    DefaultRestart,
    /// A new process's graceful stop signal.
    #[strum(props(agent_may_set = true))]
    DefaultStopSignal,
    /// A new process's grace period before force-kill.
    #[strum(props(agent_may_set = true))]
    DefaultStopGrace,
    /// Whether an agent over MCP may edit the stack.
    McpStackEdits,
    /// How long a project with no shell in it keeps running.
    IdleStop,
}

impl SettingKey {
    /// Whether an agent over MCP may change this setting.
    pub fn agent_may_set(self) -> bool {
        self.get_bool(AGENT_MAY_SET).unwrap_or_default()
    }
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
        assert_eq!(SettingKey::McpStackEdits.to_string(), "mcp-stack-edits");
        assert_eq!(SettingKey::IdleStop.to_string(), "idle-stop");
    }

    /// An agent sets exactly the `default-*` keys.
    #[test]
    fn an_agent_sets_only_the_default_keys() {
        for key in SettingKey::iter() {
            assert_eq!(
                key.agent_may_set(),
                key.to_string().starts_with("default-"),
                "{key}"
            );
        }
    }
}
