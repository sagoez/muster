use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsString,
};

use super::{
    config_membership::ConfigMembership, launch_purpose::LaunchPurpose,
    pending_intent::PendingIntent, pending_launch::PendingLaunch,
    spawn_generation::SpawnGeneration,
};
use crate::domain::{pty::ProcessOutput, value::PaneId};

/// The launches and adoptions in flight, and the environment each pane's
/// launches run with.
#[derive(Default)]
pub(crate) struct LaunchBook {
    /// Launches and adoptions handed to a worker and not yet settled, keyed by
    /// the starting pane.
    pending: HashMap<PaneId, PendingLaunch>,
    /// Set by a shutdown: from then on the result of a launch the shutdown
    /// superseded is detached, never stopped, since the service is letting go
    /// of every process.
    detaching: bool,
    /// The environment of the command that last started each pane, which its
    /// policy restarts are launched with too.
    environments: HashMap<PaneId, BTreeMap<OsString, OsString>>,
}

impl LaunchBook {
    /// Records `launch` as `pane`'s launch in flight, replacing any other.
    pub(super) fn begin(&mut self, pane: PaneId, launch: PendingLaunch) {
        self.pending.insert(pane, launch);
    }

    /// Whether no launch is in flight.
    pub(super) fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// How many launches are in flight.
    pub(super) fn in_flight(&self) -> usize {
        self.pending.len()
    }

    /// Whether `pane` has a launch or adoption in flight.
    pub(super) fn is_starting(&self, pane: PaneId) -> bool {
        self.pending.contains_key(&pane)
    }

    /// Whether `pane` has a new process being launched, as opposed to a
    /// holder being adopted.
    pub(super) fn is_spawning(&self, pane: PaneId) -> bool {
        self.pending
            .get(&pane)
            .is_some_and(|launch| matches!(launch.purpose(), LaunchPurpose::Spawn { .. }))
    }

    /// Keeps `output` for replay when it belongs to `pane`'s launch at
    /// `generation`; otherwise hands it back.
    pub(super) fn keep_early(
        &mut self,
        pane: PaneId,
        generation: SpawnGeneration,
        output: ProcessOutput,
    ) -> Option<ProcessOutput> {
        match self
            .pending
            .get_mut(&pane)
            .filter(|launch| launch.generation() == generation)
        {
            Some(launch) => {
                launch.keep(output);
                None
            },
            None => Some(output),
        }
    }

    /// Marks `pane`'s launch of a new process, if any, as a restart.
    pub(super) fn mark_restarting(&mut self, pane: PaneId) {
        if let Some(launch) = self.pending.get_mut(&pane) {
            launch.mark_restarting();
        }
    }

    /// Records `intent` for `pane`'s launch to apply once it settles: until
    /// then there is no process to signal, and superseding the launch could
    /// leave an adopted child running behind a row that claims it stopped.
    /// Returns whether `pane` is starting, and so recorded it.
    pub(super) fn record_intent(&mut self, pane: PaneId, intent: PendingIntent) -> bool {
        self.pending
            .get_mut(&pane)
            .map(|launch| launch.record_intent(intent))
            .is_some()
    }

    /// Records a pause (`pause`) or resume for `pane`'s launch to apply once
    /// it settles. Returns whether `pane` is starting, and so recorded it.
    pub(super) fn record_suspend(&mut self, pane: PaneId, pause: bool) -> bool {
        self.pending
            .get_mut(&pane)
            .map(|launch| launch.record_suspend(pause))
            .is_some()
    }

    /// Records `membership` for `pane`'s launch to take once it settles.
    pub(super) fn record_membership(&mut self, pane: PaneId, membership: ConfigMembership) {
        if let Some(launch) = self.pending.get_mut(&pane) {
            launch.record_membership(membership);
        }
    }

    /// Takes `pane`'s launch when it is the one at `generation`; a newer one
    /// is left in place.
    pub(super) fn take(
        &mut self,
        pane: PaneId,
        generation: SpawnGeneration,
    ) -> Option<PendingLaunch> {
        let recorded = self
            .pending
            .get(&pane)
            .is_some_and(|launch| launch.generation() == generation);
        if recorded {
            self.pending.remove(&pane)
        } else {
            None
        }
    }

    /// Drops `pane`'s launch, whatever its generation.
    pub(super) fn cancel(&mut self, pane: PaneId) {
        self.pending.remove(&pane);
    }

    /// From now on, detaches the result of every superseded launch.
    pub(super) fn detach_all(&mut self) {
        self.detaching = true;
    }

    /// Whether a shutdown is letting go of every process.
    pub(super) fn is_detaching(&self) -> bool {
        self.detaching
    }

    /// Takes `environment` as the one `pane`'s launches run with.
    pub(super) fn set_environment(
        &mut self,
        pane: PaneId,
        environment: BTreeMap<OsString, OsString>,
    ) {
        self.environments.insert(pane, environment);
    }

    /// The environment `pane`'s launches run with, empty when none was
    /// given.
    pub(super) fn environment(&self, pane: PaneId) -> BTreeMap<OsString, OsString> {
        self.environments.get(&pane).cloned().unwrap_or_default()
    }

    /// The environment `pane`'s launches run with, when one was given.
    #[cfg(test)]
    pub(super) fn environment_of(&self, pane: PaneId) -> Option<&BTreeMap<OsString, OsString>> {
        self.environments.get(&pane)
    }

    /// Forgets `pane`'s environment, for a pane retired from the workspace.
    pub(super) fn forget(&mut self, pane: PaneId) {
        self.environments.remove(&pane);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::process::ProcessState;

    const PANE: u64 = 0;

    /// A book with a launch of a new process on [`PANE`] at the initial
    /// generation.
    fn launching() -> LaunchBook {
        let mut book = LaunchBook::default();
        book.begin(
            PaneId::new(PANE),
            PendingLaunch::new(
                SpawnGeneration::initial(),
                ProcessState::Pending,
                LaunchPurpose::Spawn { restarting: false },
            ),
        );
        book
    }

    /// Only the result of the launch in flight takes it; a stale one leaves
    /// it in place.
    #[test]
    fn a_launch_is_taken_only_at_its_generation() {
        let mut book = launching();
        let pane = PaneId::new(PANE);

        assert!(book.take(pane, SpawnGeneration::initial().next()).is_none());
        assert!(book.is_starting(pane));
        assert!(book.take(pane, SpawnGeneration::initial()).is_some());
        assert!(book.is_empty());
    }

    /// Output and asks are kept only for a pane that is starting, and output
    /// only for the launch it belongs to.
    #[test]
    fn asks_and_output_are_kept_only_while_starting() {
        let mut book = launching();
        let pane = PaneId::new(PANE);
        let other = PaneId::new(PANE + 1);
        let chunk = || ProcessOutput::Chunk(b"x".to_vec());

        assert!(book.record_intent(pane, PendingIntent::Stop));
        assert!(!book.record_intent(other, PendingIntent::Stop));
        assert!(
            book.keep_early(pane, SpawnGeneration::initial(), chunk())
                .is_none()
        );
        assert!(
            book.keep_early(pane, SpawnGeneration::initial().next(), chunk())
                .is_some()
        );
        let launch = book.take(pane, SpawnGeneration::initial()).unwrap();
        assert_eq!(launch.intent(), Some(PendingIntent::Stop));
        assert_eq!(launch.into_early(), vec![chunk()]);
    }
}
