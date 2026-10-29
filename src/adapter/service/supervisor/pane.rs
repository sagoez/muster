use std::time::Instant;

use getset::{CopyGetters, Getters, MutGetters};
use typed_builder::TypedBuilder;

use super::{
    config_membership::ConfigMembership, shutdown_generation::ShutdownGeneration,
    unsupervised_child::UnsupervisedChild,
};
use crate::{
    adapter::terminal::{ActivityTracker, KittyKeyboardTracker, SignalReader, TerminalEmulator},
    domain::{notification::NotificationScope, port::ProcessHandle, process::ExitIntent},
};

/// One supervised pane: its terminal and, while alive, its process handle. The
/// terminal outlives the handle so a finished process keeps its last screen.
#[derive(Getters, MutGetters, CopyGetters, TypedBuilder)]
pub(crate) struct Pane {
    /// The pane's screen and scrollback.
    #[getset(get = "pub(crate)", get_mut = "pub(crate)")]
    pub(super) parser: TerminalEmulator,
    /// Unique scope for Kitty identifiers emitted by this terminal lifetime.
    pub(super) notification_scope: NotificationScope,
    /// Decodes notification and progress signals from the output stream.
    pub(super) signals: SignalReader,
    /// The Kitty keyboard protocol this child has negotiated.
    #[getset(get = "pub(crate)")]
    pub(super) keyboard: KittyKeyboardTracker,
    /// Evidence governing inferred working, idle, and attention transitions.
    pub(super) activity: ActivityTracker,
    /// Last bell accepted from this terminal lifetime, for burst throttling.
    pub(super) last_bell: Option<Instant>,
    /// Delayed desktop alert for a detached non-agent pane that rang its bell.
    pub(super) pending_bell_notification: Option<Instant>,
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
    /// Whether the pane still belongs to the current config.
    #[getset(get_copy = "pub(crate)")]
    pub(super) config_membership: ConfigMembership,
}
