use strum::{Display, EnumString};

/// The hook events `muster mcp guard` answers, spelled as Claude Code and
/// Codex name them in a hook's input and output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Display, EnumString)]
#[strum(serialize_all = "kebab-case")]
pub enum HookEvent {
    /// Before a tool runs: the guard may deny it.
    #[strum(serialize = "PreToolUse")]
    PreToolUse,
    /// After a tool ran: the guard may add a reminder.
    #[strum(serialize = "PostToolUse")]
    PostToolUse,
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    /// Each event reads and writes as the agents spell it, and no other
    /// event is answered.
    #[test]
    fn events_are_spelled_as_the_agents_spell_them() {
        for event in [HookEvent::PreToolUse, HookEvent::PostToolUse] {
            assert_eq!(HookEvent::from_str(&event.to_string()), Ok(event));
        }
        assert_eq!(HookEvent::PreToolUse.to_string(), "PreToolUse");
        assert!(HookEvent::from_str("SessionStart").is_err());
    }
}
