use getset::CopyGetters;
use thiserror::Error;
use typed_builder::TypedBuilder;

use crate::domain::value::LocalPort;

/// A list of ports refused for naming one port more than once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error, CopyGetters, TypedBuilder)]
#[error("port {port} is listed twice")]
pub struct RepeatedPort {
    /// The port listed again.
    #[getset(get_copy = "pub")]
    port: LocalPort,
}
