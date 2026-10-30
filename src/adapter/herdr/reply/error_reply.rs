use getset::Getters;
use serde::Deserialize;
use typed_builder::TypedBuilder;

use super::error_body::ErrorBody;

/// herdr's refusal, as it prints it on stderr:
/// `{"error": {"code", "message"}}`. Only the fields muster reads; herdr
/// may add others.
#[derive(Debug, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ErrorReply {
    /// The refusal itself.
    error: ErrorBody,
}
