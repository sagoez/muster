use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use crate::domain::{agent_session::AgentSessionState, process::AgentTool, value::ProcessName};

/// One durable agent session in this project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct SessionSummary {
    /// The agent's name.
    name: ProcessName,
    /// The provider that runs it.
    tool: AgentTool,
    /// Lifecycle of the durable record.
    state: AgentSessionState,
    /// Whether a native conversation was captured, so the session can be resumed.
    resumable: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A session round-trips and lists its provider and state as kebab-case
    /// tokens.
    #[test]
    fn a_session_summary_uses_kebab_case_tokens() {
        let summary = SessionSummary::builder()
            .name(ProcessName::try_new("Ada").unwrap())
            .tool(AgentTool::Opencode)
            .state(AgentSessionState::Pending)
            .resumable(false)
            .build();

        let json = serde_json::to_value(&summary).unwrap();

        assert_eq!(
            json,
            serde_json::json!({
                "name": "Ada",
                "tool": "opencode",
                "state": "pending",
                "resumable": false,
            })
        );
        assert_eq!(
            serde_json::from_value::<SessionSummary>(json).unwrap(),
            summary
        );
    }
}
