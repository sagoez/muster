use getset::{CopyGetters, Getters};
use schemars::JsonSchema;
use serde::Deserialize;
use typed_builder::TypedBuilder;

use super::{process_fields::choice, tool_error::ToolError};
use crate::domain::terminal_workspace::SplitDirection;

/// The `direction` field.
const DIRECTION_FIELD: &str = "direction";

/// The process to show in a new pane beside the agent's, and how.
#[derive(Debug, Deserialize, JsonSchema, Getters, CopyGetters, TypedBuilder)]
#[serde(deny_unknown_fields)]
pub struct ProcessOpenArg {
    /// The process's name, as stack_status shows it.
    #[getset(get = "pub")]
    name: String,
    /// A registered project's name or a folder; defaults to the project
    /// containing the server's working directory.
    #[builder(default)]
    #[getset(get = "pub")]
    project: Option<String>,
    /// Open it watch-only, so keys typed in the pane never reach it.
    /// Defaults to false.
    #[builder(default)]
    #[getset(get_copy = "pub")]
    read_only: Option<bool>,
    /// Which side of your pane it opens on: <split-directions>. Without it,
    /// right when the pane is wider than tall and each half keeps at least
    /// <split-min-half-cols> columns, and down otherwise.
    #[builder(default)]
    direction: Option<String>,
}

impl ProcessOpenArg {
    /// The side `direction` names, or none for the workspace to pick.
    ///
    /// # Errors
    /// Returns the refusal of a `direction` that names no side, listing
    /// every side.
    pub fn split(&self) -> Result<Option<SplitDirection>, ToolError> {
        self.direction
            .as_deref()
            .map(|raw| choice(DIRECTION_FIELD, raw))
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::client::FailureKind;

    /// The arguments naming `api` and `direction`.
    fn directed(direction: &str) -> ProcessOpenArg {
        ProcessOpenArg::builder()
            .name("api".to_string())
            .direction(Some(direction.to_string()))
            .build()
    }

    /// A known side reads as itself; none leaves the side to the workspace.
    #[test]
    fn a_direction_reads_as_its_side() {
        assert_eq!(
            directed("down").split().unwrap(),
            Some(SplitDirection::Down)
        );
        assert_eq!(
            directed(" right ").split().unwrap(),
            Some(SplitDirection::Right)
        );
        let undirected = ProcessOpenArg::builder().name("api".to_string()).build();
        assert_eq!(undirected.split().unwrap(), None);
    }

    /// Unknown fields are refused, as every tool's arguments are.
    #[test]
    fn an_unknown_field_is_refused() {
        let parsed: Result<ProcessOpenArg, _> =
            serde_json::from_value(serde_json::json!({"name": "api", "focus": true}));
        assert!(parsed.is_err());
    }

    /// An unknown side is the caller's mistake naming `direction`.
    #[test]
    fn an_unknown_direction_is_invalid_input() {
        let error = directed("left").split().unwrap_err();
        assert_eq!(error.kind(), FailureKind::InvalidInput);
        assert_eq!(
            error.to_string(),
            "invalid direction: expected right, down, got 'left'"
        );
    }
}
