use getset::{CopyGetters, Getters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::port_holder::PortHolder;
use crate::domain::value::LocalPort;

/// Why a process was not started: another supervised process listens on the
/// port its readiness check names.
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, TypedBuilder,
)]
#[serde(rename_all = "kebab-case")]
pub struct PortBlock {
    /// The port it needs.
    #[getset(get_copy = "pub")]
    port: LocalPort,
    /// Who listens on it.
    #[getset(get = "pub")]
    holder: PortHolder,
}
