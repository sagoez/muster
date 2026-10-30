use serde::Deserialize;

/// herdr's refusal, as it prints it on stderr:
/// `{"error": {"code", "message"}}`. Only the fields muster reads; herdr
/// may add others.
#[derive(Debug, Deserialize)]
pub(super) struct ErrorReply {
    /// The refusal itself.
    error: ErrorBody,
}

/// The body of an [`ErrorReply`].
#[derive(Debug, Deserialize)]
struct ErrorBody {
    /// herdr's code for the refusal.
    code: String,
    /// herdr's own words.
    message: String,
}

impl ErrorReply {
    /// The refusal's code and message, as herdr wrote them.
    pub(super) fn into_parts(self) -> (String, String) {
        (self.error.code, self.error.message)
    }
}
