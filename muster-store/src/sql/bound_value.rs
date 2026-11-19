//! One value a statement binds to a named parameter.

use getset::Getters;
use rusqlite::types::Value;
use typed_builder::TypedBuilder;

/// A value and the named parameter it is bound to.
#[derive(Clone, Debug, PartialEq, Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct BoundValue {
    /// The parameter, such as `:name`.
    param: String,
    /// The value bound to it.
    value: Value,
}
