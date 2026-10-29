use getset::CopyGetters;
use thiserror::Error;
use typed_builder::TypedBuilder;

use super::declared_ports::DECLARED_PORTS_MAX;

/// A port set refused for naming more ports than a process may declare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error, CopyGetters, TypedBuilder)]
#[error("{found} ports are more than the {DECLARED_PORTS_MAX} a process may declare")]
pub struct TooManyPorts {
    /// How many distinct ports it would have held.
    #[getset(get_copy = "pub")]
    found: usize,
}
