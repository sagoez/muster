use std::path::PathBuf;

use getset::{CopyGetters, Getters};
use muster_protocol::holder::HolderError;
use typed_builder::TypedBuilder;

use crate::holder_client::pool::sweep_target::SweepTarget;

/// One thing a sweep of a holder sockets folder could not do.
#[derive(Debug, Getters, CopyGetters, TypedBuilder)]
pub struct SweepFailure {
    /// What was being swept.
    #[getset(get_copy = "pub")]
    target: SweepTarget,
    /// The folder or file it failed on.
    #[getset(get = "pub")]
    path: PathBuf,
    /// Why it failed.
    #[getset(get = "pub")]
    error: HolderError,
}
