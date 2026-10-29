use getset::{CopyGetters, Getters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::ExitOutcome;

/// What a process's earlier runs leave to the one being spawned, carried with
/// it so a service that adopts it later still knows them.
#[derive(
    Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, TypedBuilder,
)]
#[serde(rename_all = "kebab-case")]
pub struct SpawnHistory {
    /// Times the process was restarted, this spawn included when it is one.
    #[getset(get_copy = "pub")]
    restarts: u32,
    /// How the previous run ended, when there was one.
    #[getset(get = "pub")]
    last_exit: Option<ExitOutcome>,
}
