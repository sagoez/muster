use serde::Serialize;

use super::log_error::LogError;
use crate::http::{Response, Status};

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
        error: LogError,
    },
}

impl LogAnswer {
    /// The answer as a JSON response with `status`.
    pub fn response(&self, status: Status) -> Response {
        Response::json(status, serde_json::to_vec(self).unwrap_or_default())
    }
}
