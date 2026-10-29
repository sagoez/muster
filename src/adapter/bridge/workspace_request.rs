use serde::{Deserialize, Serialize};

use crate::domain::value::ProcessName;

/// A request an MCP client sends the service about one project. Input is
/// answered with `Ok` or a refusal, sessions with their listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceRequest {
    /// Write `data` to the terminal of the process `name`, as if a person
    /// typed it.
    SendInput {
        /// The process to type into.
        name: ProcessName,
        /// Bytes to write to its terminal.
        data: String,
    },
    /// Enumerate this project's durable agent sessions.
    ListSessions,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every request round-trips, with kebab-case labels on the wire.
    #[test]
    fn requests_round_trip_in_kebab_case() {
        let send = WorkspaceRequest::SendInput {
            name: ProcessName::try_new("dev server").unwrap(),
            data: "ls\r".to_string(),
        };
        for request in [send.clone(), WorkspaceRequest::ListSessions] {
            let json = serde_json::to_string(&request).unwrap();
            assert_eq!(
                serde_json::from_str::<WorkspaceRequest>(&json).unwrap(),
                request
            );
        }
        assert_eq!(
            serde_json::to_value(&send).unwrap(),
            serde_json::json!({ "send-input": { "name": "dev server", "data": "ls\r" } })
        );
        assert_eq!(
            serde_json::to_value(WorkspaceRequest::ListSessions).unwrap(),
            "list-sessions"
        );
    }
}
