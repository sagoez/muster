use getset::Getters;
use serde::Deserialize;
use typed_builder::TypedBuilder;

/// The body of an [`ErrorReply`](super::ErrorReply).
#[derive(Debug, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ErrorBody {
    /// herdr's code for the refusal.
    code: String,
    /// herdr's own words.
    message: String,
}
