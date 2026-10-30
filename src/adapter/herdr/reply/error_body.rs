use serde::Deserialize;

/// The body of an [`ErrorReply`](super::ErrorReply).
#[derive(Debug, Deserialize)]
pub struct ErrorBody {
    /// herdr's code for the refusal.
    code: String,
    /// herdr's own words.
    message: String,
}

impl ErrorBody {
    /// The code and the message, as herdr wrote them.
    pub fn into_parts(self) -> (String, String) {
        (self.code, self.message)
    }
}
