//! Tool descriptions that state the bounds a call runs under, read from the
//! constants that enforce them, so a description never drifts from the code.
//! A description names a bound by placeholder, since the tool macro takes
//! only a literal.

use rmcp::handler::server::router::tool::ToolRouter;

use super::{
    bounds::{CALL_BOUND, DEFAULT_LIST_LIMIT, DEFAULT_WAIT, MAX_LIST_LIMIT},
    coordination_tools::LISTED_VALUE_CHARS,
};
use crate::{
    adapter::{
        holder::{DEFAULT_LOG_BYTES, DEFAULT_LOG_LINES, MAX_LOG_BYTES, MAX_LOG_LINES},
        service::{EVENTS_WITHOUT_SINCE, supervisor::FAILURE_TAIL_LINES},
    },
    domain::coordination::{
        KV_VALUE_MAX_CHARS, LOCK_TTL_DEFAULT, LOCK_TTL_MAX, NOTE_BODY_MAX_CHARS,
        TODO_LEASE_DEFAULT, TODO_LEASE_MAX, TODO_TITLE_MAX_CHARS,
    },
};

/// The placeholders a description may hold, each with the value it reads.
fn placeholders() -> [(&'static str, String); 18] {
    [
        ("<listed-value-chars>", LISTED_VALUE_CHARS.to_string()),
        ("<note-body-max-chars>", NOTE_BODY_MAX_CHARS.to_string()),
        ("<kv-value-max-chars>", KV_VALUE_MAX_CHARS.to_string()),
        ("<todo-title-max-chars>", TODO_TITLE_MAX_CHARS.to_string()),
        ("<call-bound-s>", CALL_BOUND.as_secs().to_string()),
        ("<default-wait-s>", DEFAULT_WAIT.as_secs().to_string()),
        ("<failure-tail-lines>", FAILURE_TAIL_LINES.to_string()),
        ("<default-log-lines>", DEFAULT_LOG_LINES.to_string()),
        ("<max-log-lines>", MAX_LOG_LINES.to_string()),
        ("<default-log-bytes>", DEFAULT_LOG_BYTES.to_string()),
        ("<max-log-bytes>", MAX_LOG_BYTES.to_string()),
        (
            "<default-lease-s>",
            TODO_LEASE_DEFAULT.as_secs().to_string(),
        ),
        ("<max-lease-s>", TODO_LEASE_MAX.as_secs().to_string()),
        ("<default-ttl-s>", LOCK_TTL_DEFAULT.as_secs().to_string()),
        ("<max-ttl-s>", LOCK_TTL_MAX.as_secs().to_string()),
        ("<default-list-limit>", DEFAULT_LIST_LIMIT.to_string()),
        ("<max-list-limit>", MAX_LIST_LIMIT.to_string()),
        ("<events-without-since>", EVENTS_WITHOUT_SINCE.to_string()),
    ]
}

/// `router` with every tool description's placeholders replaced by the
/// values they name.
pub fn described<S>(mut router: ToolRouter<S>) -> ToolRouter<S> {
    for route in router.map.values_mut() {
        if let Some(template) = &route.attr.description {
            route.attr.description = Some(resolved(template).into());
        }
    }
    router
}

/// `template` with each placeholder replaced by the value it names.
fn resolved(template: &str) -> String {
    placeholders()
        .iter()
        .fold(template.to_string(), |text, (placeholder, value)| {
            text.replace(placeholder, value)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::mcp::test_support::fixture;

    #[test]
    fn a_placeholder_reads_its_constant() {
        assert_eq!(
            resolved("at most <call-bound-s> seconds, last <failure-tail-lines> lines"),
            format!(
                "at most {} seconds, last {FAILURE_TAIL_LINES} lines",
                CALL_BOUND.as_secs()
            )
        );
    }

    /// Every tool's description is fully resolved: no placeholder is left.
    #[test]
    fn no_description_keeps_a_placeholder() {
        let fixture = fixture();

        for tool in fixture.server.tool_router.list_all() {
            let description = tool.description.unwrap_or_default();
            assert!(
                !placeholders()
                    .iter()
                    .any(|(placeholder, _)| description.contains(placeholder))
                    && !description.contains("<"),
                "{}: {description}",
                tool.name
            );
        }
    }
}
