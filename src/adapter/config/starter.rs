use crate::domain::{
    config::{ProcessSpec, WorkspaceConfig},
    value::ProcessName,
};

/// Name of the starter terminal created for a new project.
const STARTER_TERMINAL: &str = "Terminal";

/// A starter workspace for a new project: a single terminal running the login
/// shell, so the project is immediately usable. Shared by the TUI's new-project
/// flow and `muster init` so the two cannot drift.
pub fn starter_workspace() -> WorkspaceConfig {
    let terminals = ProcessName::try_new(STARTER_TERMINAL)
        .map(|name| vec![ProcessSpec::builder().name(name).build()])
        .unwrap_or_default();
    WorkspaceConfig::builder()
        .agents(vec![])
        .terminals(terminals)
        .commands(vec![])
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The starter is exactly one terminal and nothing else.
    #[test]
    fn starter_is_a_single_login_shell_terminal() {
        let config = starter_workspace();
        assert_eq!(config.terminals().len(), 1);
        assert_eq!(config.terminals()[0].name().as_ref(), STARTER_TERMINAL);
        assert!(config.agents().is_empty() && config.commands().is_empty());
    }
}
