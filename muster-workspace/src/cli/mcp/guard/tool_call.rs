use std::fmt::{self, Display, Formatter};

use getset::{CopyGetters, Getters};
use muster_domain::text::{ListForm, ValueList};
use muster_mcp::ToolName;
use typed_builder::TypedBuilder;

/// Opens a call's arguments.
const CALL_OPEN: &str = "(";
/// Closes a call's arguments.
const CALL_CLOSE: &str = ")";
/// Joins an argument's name to its value.
const ARG_IS: &str = "=";

/// A call of one of muster's MCP tools, as a refusal tells an agent to make
/// it: `tool(name=value, ...)`, its arguments added one at a time.
#[derive(Clone, Debug, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct ToolCall {
    /// The tool called.
    #[getset(get_copy = "pub")]
    tool: ToolName,
    /// Its arguments in order, each written `name=value`.
    #[getset(get = "pub")]
    #[builder(default)]
    args: Vec<String>,
}

impl ToolCall {
    /// A call of `tool` with no arguments yet.
    pub fn of(tool: ToolName) -> Self {
        Self::builder().tool(tool).build()
    }

    /// This call with the argument `name` set to `value` after the others.
    #[must_use]
    pub fn arg(mut self, name: &str, value: &str) -> Self {
        self.args.push(format!("{name}{ARG_IS}{value}"));
        self
    }
}

impl Display for ToolCall {
    /// Writes the call as an agent writes it.
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}{CALL_OPEN}{}{CALL_CLOSE}",
            self.tool(),
            ValueList::of_values(self.args()).written(ListForm::Commas)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A call reads as an agent writes it, its arguments in the order added.
    #[test]
    fn a_call_reads_as_an_agent_writes_it() {
        let call = ToolCall::of(ToolName::TodoClaim)
            .arg("project", "\"/work/shop\"")
            .arg("lease_s", "28800");
        assert_eq!(
            call.to_string(),
            "todo_claim(project=\"/work/shop\", lease_s=28800)"
        );
        assert_eq!(
            ToolCall::of(ToolName::StackStatus).to_string(),
            "stack_status()"
        );
    }
}
