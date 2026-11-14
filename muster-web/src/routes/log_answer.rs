use serde::Serialize;

use crate::http::{Response, Status};

/// What the log route says of a process the floor does not hold.
pub const UNKNOWN_PROCESS: &str = "unknown process";
/// What the log route says of a log it could not read.
pub const UNREADABLE_LOG: &str = "unreadable log";

/// The log route's JSON: a process's last lines, or why there are none.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum LogAnswer {
    /// The last lines, oldest first.
    Lines {
        /// Each line, as plain text.
        lines: Vec<String>,
    },
    /// Why no lines were read.
    Error {
        /// The reason.
        error: &'static str,
    },
}

impl LogAnswer {
    /// The answer as a JSON response with `status`.
    pub fn response(&self, status: Status) -> Response {
        Response::json(status, serde_json::to_vec(self).unwrap_or_default())
    }
}
