use std::time::Duration;

use schemars::JsonSchema;
use serde::Deserialize;

use super::{condition_arg::ConditionArg, tool_error::ToolError};
use crate::adapter::service::WaitSpec;

/// The `wait` field of a start or restart.
const WAIT_FIELD: &str = "wait";

/// Whether a start or restart waits: true for the process's default
/// condition, or a condition as process_wait takes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum WaitArg {
    /// true waits for readiness, or running without a readiness check.
    Flag(bool),
    /// Waits for this condition.
    Until(ConditionArg),
}

impl WaitArg {
    /// The wait this asks for, bounded by `timeout`, or none for `false`.
    ///
    /// # Errors
    /// Returns the refusal of a condition out of form.
    pub fn spec(&self, timeout: Duration) -> Result<Option<WaitSpec>, ToolError> {
        let until = match self {
            Self::Flag(false) => return Ok(None),
            Self::Flag(true) => None,
            Self::Until(condition) => Some(condition.condition(WAIT_FIELD)?),
        };
        Ok(Some(
            WaitSpec::builder()
                .until(until)
                .since(None)
                .timeout(timeout)
                .build(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::service::WaitCondition;

    /// The bound every wait here takes.
    const BOUND: Duration = Duration::from_secs(30);

    /// The wait an agent sent as `json`.
    fn read(json: serde_json::Value) -> Option<WaitSpec> {
        serde_json::from_value::<WaitArg>(json)
            .unwrap()
            .spec(BOUND)
            .unwrap()
    }

    /// false waits for nothing, true for the default condition, and an
    /// object for that condition.
    #[test]
    fn a_flag_or_a_condition() {
        assert_eq!(read(serde_json::json!(false)), None);
        let default = read(serde_json::json!(true)).expect("a wait");
        assert_eq!(default.until(), &None);
        assert_eq!(*default.timeout(), BOUND);
        let state = read(serde_json::json!({ "state": "running" })).expect("a wait");
        assert!(matches!(state.until(), Some(WaitCondition::State(_))));
    }

    /// A condition out of form is refused under the `wait` field.
    #[test]
    fn a_bad_condition_names_the_wait_field() {
        let refused = serde_json::from_value::<WaitArg>(serde_json::json!({ "port": 0 }))
            .unwrap()
            .spec(BOUND)
            .unwrap_err();
        assert!(refused.to_string().contains("invalid wait"), "{refused}");
    }
}
