//! Tool descriptions, argument descriptions and server instructions that state
//! the bounds a call runs under, read from the constants that enforce them, so
//! a description never drifts from the code. Text names a bound by
//! placeholder, since the tool macro and doc comments take only a literal.

use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use serde_json::Value;

use super::bounds::{CALL_BOUND, DEFAULT_WAIT};
use crate::{
    adapter::{
        client::{DEFAULT_LIST_LIMIT, LISTED_VALUE_CHARS, MAX_LIST_LIMIT},
        holder::{DEFAULT_LOG_BYTES, DEFAULT_LOG_LINES, MAX_LOG_BYTES, MAX_LOG_LINES},
        service::{EVENTS_WITHOUT_SINCE, supervisor::FAILURE_TAIL_LINES},
    },
    domain::coordination::{
        KV_VALUE_MAX_CHARS, LOCK_NAME_MAX_CHARS, LOCK_TTL_DEFAULT, LOCK_TTL_MAX,
        NOTE_BODY_MAX_CHARS, TODO_LEASE_DEFAULT, TODO_LEASE_MAX, TODO_TITLE_MAX_CHARS,
    },
};

/// Key of a JSON schema node's description.
const DESCRIPTION_KEY: &str = "description";

/// The placeholders a description may hold, each with the value it reads.
fn placeholders() -> [(&'static str, String); 19] {
    [
        ("<listed-value-chars>", LISTED_VALUE_CHARS.to_string()),
        ("<note-body-max-chars>", NOTE_BODY_MAX_CHARS.to_string()),
        ("<kv-value-max-chars>", KV_VALUE_MAX_CHARS.to_string()),
        ("<todo-title-max-chars>", TODO_TITLE_MAX_CHARS.to_string()),
        ("<lock-name-max-chars>", LOCK_NAME_MAX_CHARS.to_string()),
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

/// `router` with the placeholders in every tool description and every
/// description of its input schema replaced by the values they name.
pub fn described<S>(mut router: ToolRouter<S>) -> ToolRouter<S> {
    for route in router.map.values_mut() {
        if let Some(template) = &route.attr.description {
            route.attr.description = Some(resolved(template).into());
        }
        let mut schema = Value::Object(route.attr.input_schema.as_ref().clone());
        resolve_descriptions(&mut schema);
        if let Value::Object(schema) = schema {
            route.attr.input_schema = Arc::new(schema);
        }
    }
    router
}

/// Replaces the placeholders in every description string within `node`.
fn resolve_descriptions(node: &mut Value) {
    match node {
        Value::Object(fields) => {
            for (key, value) in fields.iter_mut() {
                match value {
                    Value::String(text) if key == DESCRIPTION_KEY => *text = resolved(text),
                    _ => resolve_descriptions(value),
                }
            }
        },
        Value::Array(items) => items.iter_mut().for_each(resolve_descriptions),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {},
    }
}

/// `template` with each placeholder replaced by the value it names.
pub(super) fn resolved(template: &str) -> String {
    placeholders()
        .iter()
        .fold(template.to_string(), |text, (placeholder, value)| {
            text.replace(placeholder, value)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::mcp::{server::instructions, test_support::fixture};

    /// Whether `text` still holds a placeholder.
    fn keeps_a_placeholder(text: &str) -> bool {
        placeholders()
            .iter()
            .any(|(placeholder, _)| text.contains(placeholder))
            || text.contains('<')
    }

    /// Every description string within `node`.
    fn descriptions(node: &Value) -> Vec<String> {
        match node {
            Value::Object(fields) => fields
                .iter()
                .flat_map(|(key, value)| match value {
                    Value::String(text) if key == DESCRIPTION_KEY => vec![text.clone()],
                    _ => descriptions(value),
                })
                .collect(),
            Value::Array(items) => items.iter().flat_map(descriptions).collect(),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => Vec::new(),
        }
    }

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

    /// Every tool's description, every description in its input schema and
    /// the server instructions are fully resolved: no placeholder is left.
    #[test]
    fn no_description_keeps_a_placeholder() {
        let fixture = fixture();

        for tool in fixture.server.tool_router.list_all() {
            let description = tool.description.clone().unwrap_or_default();
            assert!(
                !keeps_a_placeholder(&description),
                "{}: {description}",
                tool.name
            );
            let schema = Value::Object(tool.input_schema.as_ref().clone());
            for description in descriptions(&schema) {
                assert!(
                    !keeps_a_placeholder(&description),
                    "{} input: {description}",
                    tool.name
                );
            }
        }
        let instructions = instructions();
        assert!(!keeps_a_placeholder(&instructions), "{instructions}");
    }

    /// A bound named in an argument's description reaches the schema as the
    /// value of its constant.
    #[test]
    fn an_argument_description_states_its_bound() {
        let fixture = fixture();
        let wait = fixture
            .server
            .tool_router
            .list_all()
            .into_iter()
            .find(|tool| tool.name == "process_wait")
            .unwrap();

        let schema = Value::Object(wait.input_schema.as_ref().clone());
        assert!(
            descriptions(&schema)
                .iter()
                .any(|text| text.contains(&format!("at most {}", CALL_BOUND.as_secs()))),
            "{schema}"
        );
    }
}
