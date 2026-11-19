//! The tools an agent has from the start of its session, rather than only
//! once a tool search finds them.

use std::str::FromStr;

use rmcp::{handler::server::router::tool::ToolRouter, model::MetaObject};
use serde_json::{Map, Value};

use crate::runtime::ToolName;

/// The `_meta` key Claude Code reads to load a tool up front instead of
/// deferring it behind tool search.
pub const ALWAYS_LOAD_KEY: &str = "anthropic/alwaysLoad";

/// `router` with the always-load flag on each tool [`ToolName`] marks as
/// loaded up front: the reads that tell an agent what runs and what other
/// agents left behind, and the note write that leaves what it learned.
pub fn loaded_up_front<S>(mut router: ToolRouter<S>) -> ToolRouter<S> {
    for route in router.map.values_mut() {
        if ToolName::from_str(&route.attr.name).is_ok_and(ToolName::loaded_up_front) {
            route.attr.meta = Some(MetaObject(Map::from_iter([(
                ALWAYS_LOAD_KEY.to_owned(),
                Value::Bool(true),
            )])));
        }
    }
    router
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::test_support::fixture;

    /// The tools [`ToolName`] marks carry the always-load flag and no other
    /// tool does, so a manual registration loads them up front without
    /// loading every tool.
    #[test]
    fn only_the_marked_tools_load_up_front() {
        let fixture = fixture();

        for tool in fixture.server().tool_router.list_all() {
            let flagged = tool
                .meta
                .as_ref()
                .and_then(|meta| meta.0.get(ALWAYS_LOAD_KEY))
                == Some(&Value::Bool(true));
            let marked = ToolName::from_str(&tool.name).is_ok_and(ToolName::loaded_up_front);
            assert_eq!(flagged, marked, "{}", tool.name);
        }
        let listed = serde_json::to_value(
            fixture
                .server()
                .tool_router
                .get(ToolName::NoteRead.as_ref())
                .unwrap()
                .clone(),
        )
        .unwrap();
        assert_eq!(listed["_meta"][ALWAYS_LOAD_KEY], Value::Bool(true));
    }
}
