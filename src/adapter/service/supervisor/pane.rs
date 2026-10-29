use std::time::Instant;

use getset::{CopyGetters, Getters, MutGetters};
use typed_builder::TypedBuilder;

use super::{
    config_membership::ConfigMembership, shutdown_generation::ShutdownGeneration,
    unsupervised_child::UnsupervisedChild,
};
use crate::domain::{port::ProcessHandle, process::ExitIntent};

/// One supervised pane: its last known screen and, while alive, its process
/// handle. The screen outlives the handle so a finished process keeps it.
#[derive(Getters, MutGetters, CopyGetters, TypedBuilder)]
pub(crate) struct Pane {
    /// The text of the last screen the process showed: as it connected, as
    /// its holder last repainted it, or as a holder read last returned it.
    #[getset(get = "pub(crate)")]
    pub(super) screen: String,
    /// The live process, while there is one.
    #[getset(get = "pub(crate)", get_mut = "pub(crate)")]
    pub(super) handle: Option<Box<dyn ProcessHandle>>,
    /// The child of a process left running under a holder this service cannot
    /// reach, while the process is unsupervised.
    pub(super) unsupervised: Option<UnsupervisedChild>,
    /// When the live process started or was adopted.
    pub(super) started_at: Instant,
    /// What the pane's next exit means.
    #[getset(get_copy = "pub(crate)")]
    pub(super) exit_intent: ExitIntent,
    /// The current graceful stop request, so a stale deadline is ignored.
    pub(super) shutdown_generation: ShutdownGeneration,
    /// Whether the pane still belongs to the project's stack.
    #[getset(get_copy = "pub(crate)")]
    pub(super) config_membership: ConfigMembership,
}
