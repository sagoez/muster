use std::time::Instant;

use getset::{CopyGetters, Getters, MutGetters, Setters};
use typed_builder::TypedBuilder;

use super::{
    carried_pane::CarriedPane, config_membership::ConfigMembership,
    shutdown_generation::ShutdownGeneration, unsupervised_child::UnsupervisedChild,
};
use crate::domain::{
    port::ProcessHandle,
    process::{ExitIntent, ProcessState},
};

/// One supervised pane: its last known screen and, while alive, its process
/// handle. The screen outlives the handle so a finished process keeps it.
#[derive(Getters, MutGetters, CopyGetters, Setters, TypedBuilder)]
pub(crate) struct Pane {
    /// The text of the last screen the process showed: as it connected, as
    /// its holder last repainted it, or as a holder read last returned it.
    #[getset(get = "pub(crate)", set = "pub(super)")]
    screen: String,
    /// The live process, while there is one.
    #[getset(get = "pub(crate)", get_mut = "pub(crate)")]
    handle: Option<Box<dyn ProcessHandle>>,
    /// The child of a process left running under a holder this service cannot
    /// reach, while the process is unsupervised.
    #[getset(get_copy = "pub(super)", get_mut = "pub(super)", set = "pub(super)")]
    unsupervised: Option<UnsupervisedChild>,
    /// When the live process started or was adopted.
    #[getset(get_copy = "pub(super)", set = "pub(super)")]
    started_at: Instant,
    /// What the pane's next exit means.
    #[getset(get_copy = "pub(crate)", set = "pub(super)")]
    exit_intent: ExitIntent,
    /// The current graceful stop request, so a stale deadline is ignored.
    #[getset(get_copy = "pub(super)")]
    shutdown_generation: ShutdownGeneration,
    /// Whether the pane still belongs to the project's stack.
    #[getset(get_copy = "pub(crate)", set = "pub(super)")]
    config_membership: ConfigMembership,
}

impl Pane {
    /// Takes the live process's handle, leaving the pane without one.
    pub(super) fn take_handle(&mut self) -> Option<Box<dyn ProcessHandle>> {
        self.handle.take()
    }

    /// Supersedes the current graceful stop, so its deadline is stale, and
    /// returns the new one.
    pub(super) fn advance_shutdown(&mut self) -> ShutdownGeneration {
        self.shutdown_generation = self.shutdown_generation.next();
        self.shutdown_generation
    }

    /// Applies `change` to what the pane's next exit means.
    pub(super) fn change_intent(&mut self, change: impl FnOnce(ExitIntent) -> ExitIntent) {
        self.exit_intent = change(self.exit_intent);
    }

    /// Lets go of the live connection, returning what the pane keeps across
    /// a reconnect, with `state` as the state before it broke. None when the
    /// pane has no live connection.
    pub(super) fn carry_over(&mut self, state: Option<ProcessState>) -> Option<CarriedPane> {
        let handle = self.handle.take()?;
        Some(
            CarriedPane::builder()
                .intent(self.exit_intent)
                .membership(self.config_membership)
                .started_at(self.started_at)
                .state(state)
                .pid(handle.process_id())
                .build(),
        )
    }

    /// Takes back what `carried` kept across a reconnect.
    pub(super) fn restore(&mut self, carried: CarriedPane) {
        self.exit_intent = carried.intent();
        self.config_membership = carried.membership();
        self.started_at = carried.started_at();
    }
}
