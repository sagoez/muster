use rmcp::model::CallToolResult;

use super::tool_failure::ToolFailure;

/// What a tool answers: its result, or why it has none.
pub type ToolResult = Result<CallToolResult, ToolFailure>;
