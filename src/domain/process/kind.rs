use serde::{Deserialize, Serialize};
use strum::Display;

/// Which sidebar section a process belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display, Serialize, Deserialize)]
#[strum(serialize_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum ProcessKind {
    /// A CLI coding agent (Claude Code, Codex, ...).
    Agent,
    /// A plain interactive shell.
    Terminal,
    /// A long-running dev command (dev server, queue worker, ...).
    Command,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displays_lowercase() {
        assert_eq!(ProcessKind::Agent.to_string(), "agent");
        assert_eq!(ProcessKind::Terminal.to_string(), "terminal");
        assert_eq!(ProcessKind::Command.to_string(), "command");
    }
}
