//! The tools an agent has from the start of its session, rather than only
//! once a tool search finds them.

use rmcp::model::MetaObject;
use serde_json::{Map, Value};

/// The `_meta` key Claude Code reads to load a tool up front instead of
/// deferring it behind tool search.
pub const ALWAYS_LOAD_KEY: &str = "anthropic/alwaysLoad";

/// The `_meta` of a tool an agent should hold from the start of its session:
/// the reads that tell it what runs and what other agents left behind.
pub fn always_loaded() -> MetaObject {
    MetaObject(Map::from_iter([(
        ALWAYS_LOAD_KEY.to_owned(),
        Value::Bool(true),
    )]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::test_support::fixture;

    /// The tools every session starts with.
    const ALWAYS_LOADED: [&str; 5] = [
        "note_read",
        "todo_list",
        "stack_status",
        "process_logs",
        "project_list",
    ];

    /// The core reads carry the always-load flag and no other tool does, so
    /// a manual registration loads them up front without loading every tool.
    #[test]
    fn only_the_core_reads_load_up_front() {
        let fixture = fixture();

        for tool in fixture.server().tool_router.list_all() {
            let flagged = tool
                .meta
                .as_ref()
                .and_then(|meta| meta.0.get(ALWAYS_LOAD_KEY))
                == Some(&Value::Bool(true));
            assert_eq!(
                flagged,
                ALWAYS_LOADED.contains(&tool.name.as_ref()),
                "{}",
                tool.name
            );
        }
        let listed = serde_json::to_value(
            fixture
                .server()
                .tool_router
                .get("note_read")
                .unwrap()
                .clone(),
        )
        .unwrap();
        assert_eq!(listed["_meta"][ALWAYS_LOAD_KEY], Value::Bool(true));
    }
}
