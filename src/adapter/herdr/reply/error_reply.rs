use serde::Deserialize;

use super::error_body::ErrorBody;

/// herdr's refusal, as it prints it on stderr:
/// `{"error": {"code", "message"}}`. Only the fields muster reads; herdr
/// may add others.
#[derive(Debug, Deserialize)]
pub struct ErrorReply {
    /// The refusal itself.
    error: ErrorBody,
}

impl ErrorReply {
    /// The refusal's code and message, as herdr wrote them.
    pub fn into_parts(self) -> (String, String) {
        self.error.into_parts()
    }
}
