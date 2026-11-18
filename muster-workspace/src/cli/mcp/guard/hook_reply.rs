//! The JSON a hook prints to answer its event, in the shape Claude Code and
//! Codex both read.

use serde_json::json;

use crate::cli::mcp::guard::hook_event::HookEvent;

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
/// The key of text added to the agent's context after a tool ran.
const ADDITIONAL_CONTEXT: &str = "additionalContext";

/// The PreToolUse answer refusing the call for `reason`.
pub fn denial(reason: &str) -> String {
    json!({
        SPECIFIC_OUTPUT: {
            EVENT_NAME: HookEvent::PreToolUse.to_string(),
            DECISION: DENY,
            DECISION_REASON: reason,
        }
    })
    .to_string()
}

/// The PostToolUse answer adding `text` to the agent's context.
pub fn added_context(text: &str) -> String {
    json!({
        SPECIFIC_OUTPUT: {
            EVENT_NAME: HookEvent::PostToolUse.to_string(),
            ADDITIONAL_CONTEXT: text,
        }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    /// A denial and a reminder are the hook JSON both agents read.
    #[test]
    fn answers_are_the_agents_hook_json() {
        let denial: Value = serde_json::from_str(&denial("why")).unwrap();
        assert_eq!(
            denial,
            json!({"hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": "why",
            }})
        );
        let added: Value = serde_json::from_str(&added_context("note")).unwrap();
        assert_eq!(
            added,
            json!({"hookSpecificOutput": {
                "hookEventName": "PostToolUse",
                "additionalContext": "note",
            }})
        );
    }
}
