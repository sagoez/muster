use getset::{CopyGetters, Getters};
use muster_domain::{pty::ExitOutcome, value::OutputCursor};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

/// The exit a holder that no supervisor could reach leaves beside its
/// record for the next service, with where its child's run began in the
/// log, so that service reads the run's own output.
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, TypedBuilder,
)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct LeftExit {
    /// How the child exited.
    #[getset(get = "pub")]
    #[serde(with = "crate::wire::form")]
    outcome: ExitOutcome,
    /// Where the child's run began in its log.
    #[getset(get_copy = "pub")]
    #[serde(with = "crate::wire::form")]
    run_start: OutputCursor,
}
