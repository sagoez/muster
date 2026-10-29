use getset::CopyGetters;
use thiserror::Error;
use typed_builder::TypedBuilder;

use super::environment::ENV_VARS_MAX;

/// An environment refused for naming more variables than a process may have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error, CopyGetters, TypedBuilder)]
#[error("{found} environment variables are more than the {ENV_VARS_MAX} a process may have")]
pub struct TooManyVars {
    /// How many variables it would have held.
    #[getset(get_copy = "pub")]
    found: usize,
}
