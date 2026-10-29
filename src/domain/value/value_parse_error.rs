use getset::Getters;
use thiserror::Error;
use typed_builder::TypedBuilder;

/// Text that does not read as the value it was meant to be.
#[derive(Debug, Clone, PartialEq, Eq, Error, Getters, TypedBuilder)]
#[error("expected {expected}, got '{found}'")]
#[getset(get = "pub")]
pub struct ValueParseError {
    /// The accepted form, such as `ROWSxCOLS`.
    #[builder(setter(into))]
    expected: String,
    /// The text that was refused.
    #[builder(setter(into))]
    found: String,
}
