use getset::CopyGetters;

use super::{
    config_membership::ConfigMembership, launch_purpose::LaunchPurpose,
    pending_intent::PendingIntent, spawn_generation::SpawnGeneration,
};
use crate::domain::{process::ProcessState, pty::ProcessOutput};

/// Most output events kept for a starting pane before its result arrives.
/// Past it the oldest chunk is dropped: the screen the connection began with
/// is exact, and a dropped chunk is still in the process's log.
pub(super) const EARLY_OUTPUT_CAP: usize = 256;

/// A launch or adoption handed to a worker, from the moment the pane starts
/// until its result is settled on the loop.
#[derive(Debug, CopyGetters)]
pub(crate) struct PendingLaunch {
    /// The spawn generation the result must still match to be installed.
    #[getset(get_copy = "pub(super)")]
    generation: SpawnGeneration,
    /// The state before the pane started, restored when an adoption fails.
    #[getset(get_copy = "pub(super)")]
    prior_state: ProcessState,
    /// Why the pane is starting.
    #[getset(get_copy = "pub(super)")]
    purpose: LaunchPurpose,
    /// Output the new connection delivered before its result arrived,
    /// replayed in order once the process is installed; at most
    /// [`EARLY_OUTPUT_CAP`] events.
    early: Vec<ProcessOutput>,
    /// The latest stop, kill or restart asked for while starting.
    #[getset(get_copy = "pub(super)")]
    intent: Option<PendingIntent>,
    /// The latest pause (`true`) or resume (`false`) asked for while starting.
    #[getset(get_copy = "pub(super)")]
    suspend: Option<bool>,
    /// The config membership a reconcile gave the pane while starting.
    #[getset(get_copy = "pub(super)")]
    membership: Option<ConfigMembership>,
}

impl PendingLaunch {
    /// A launch at `generation` from `prior_state` for `purpose`, with nothing
    /// recorded yet.
    pub(super) fn new(
        generation: SpawnGeneration,
        prior_state: ProcessState,
        purpose: LaunchPurpose,
    ) -> Self {
        Self {
            generation,
            prior_state,
            purpose,
            early: Vec::new(),
            intent: None,
            suspend: None,
            membership: None,
        }
    }

    /// Keeps `output` for replay. Once [`EARLY_OUTPUT_CAP`] events are kept,
    /// the oldest chunk, warning or attach count makes room; an exit or
    /// broken stream is never dropped.
    pub(super) fn keep(&mut self, output: ProcessOutput) {
        if self.early.len() >= EARLY_OUTPUT_CAP
            && let Some(oldest) = self.early.iter().position(|kept| {
                matches!(
                    kept,
                    ProcessOutput::Chunk(_)
                        | ProcessOutput::Warning(_)
                        | ProcessOutput::Attached(_)
                )
            })
        {
            self.early.remove(oldest);
        }
        self.early.push(output);
    }

    /// Takes the output kept for replay, in order.
    pub(super) fn into_early(self) -> Vec<ProcessOutput> {
        self.early
    }

    /// Marks a launch of a new process as a restart, which `ps` counts.
    pub(super) fn mark_restarting(&mut self) {
        if let LaunchPurpose::Spawn { restarting } = &mut self.purpose {
            *restarting = true;
        }
    }

    /// Records the latest stop, kill or restart asked for while starting.
    pub(super) fn record_intent(&mut self, intent: PendingIntent) {
        self.intent = Some(intent);
    }

    /// Records the latest pause (`true`) or resume (`false`) asked for while
    /// starting.
    pub(super) fn record_suspend(&mut self, pause: bool) {
        self.suspend = Some(pause);
    }

    /// Records the config membership a reconcile gave the pane while
    /// starting.
    pub(super) fn record_membership(&mut self, membership: ConfigMembership) {
        self.membership = Some(membership);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::pty::ExitOutcome;

    /// A pending adoption with nothing kept.
    fn launch() -> PendingLaunch {
        PendingLaunch::new(
            SpawnGeneration::initial(),
            ProcessState::Pending,
            LaunchPurpose::Adopt { carried: None },
        )
    }

    /// Early output is capped by dropping the oldest chunk, never the exit.
    #[test]
    fn early_output_keeps_the_newest_chunks_and_every_exit() {
        let mut launch = launch();
        launch.keep(ProcessOutput::Exited(ExitOutcome::FAILED_UNKNOWN));
        for index in 0..=EARLY_OUTPUT_CAP {
            launch.keep(ProcessOutput::Chunk(index.to_be_bytes().to_vec()));
        }

        assert_eq!(launch.early.len(), EARLY_OUTPUT_CAP);
        assert_eq!(
            launch.early[0],
            ProcessOutput::Exited(ExitOutcome::FAILED_UNKNOWN)
        );
        assert_eq!(
            launch.early[1],
            ProcessOutput::Chunk(2usize.to_be_bytes().to_vec()),
            "the two oldest chunks made room"
        );
        assert_eq!(
            launch.early.last(),
            Some(&ProcessOutput::Chunk(
                EARLY_OUTPUT_CAP.to_be_bytes().to_vec()
            ))
        );
    }
}
