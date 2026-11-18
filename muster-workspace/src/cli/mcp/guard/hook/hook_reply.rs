//! The JSON a PreToolUse hook prints to refuse a call, in the shape Claude
//! Code and Codex both read.

use serde_json::json;

/// The event the guard answers, as both agents name it.
pub const PRE_TOOL_USE: &str = "PreToolUse";
/// The key holding an event's own answer.
const SPECIFIC_OUTPUT: &str = "hookSpecificOutput";
/// The key naming the event answered.
const EVENT_NAME: &str = "hookEventName";
/// The key of a PreToolUse decision.
const DECISION: &str = "permissionDecision";
/// The decision refusing the call.
const DENY: &str = "deny";
/// The key of the reason the agent is shown for a decision.
const DECISION_REASON: &str = "permissionDecisionReason";

/// The PreToolUse answer refusing the call for `reason`.
pub fn denial(reason: &str) -> String {
    json!({
        SPECIFIC_OUTPUT: {
            EVENT_NAME: PRE_TOOL_USE,
            DECISION: DENY,
            DECISION_REASON: reason,
        }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    /// A denial is the hook JSON both agents read.
    #[test]
    fn a_denial_is_the_agents_hook_json() {
        let denial: Value = serde_json::from_str(&denial("why")).unwrap();
        assert_eq!(
            denial,
            json!({"hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": "why",
            }})
        );
    }
}
