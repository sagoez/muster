use std::collections::HashMap;

use crate::domain::{port::ProcessHandle, value::PaneId};

/// The writable clients attached to each live pane, and the exits held for a
/// pending restart.
#[derive(Default)]
pub(crate) struct AttachBook {
    /// Writable clients attached to each live pane's terminal.
    attached: HashMap<PaneId, u32>,
    /// The handle of each pane's exited process whose exit is not yet
    /// acknowledged, held while its restart is pending so a service that
    /// stops meanwhile leaves the exit for its successor to apply again.
    held_exits: HashMap<PaneId, Box<dyn ProcessHandle>>,
}

impl AttachBook {
    /// The writable clients attached to `pane`'s terminal.
    pub(super) fn attached(&self, pane: PaneId) -> u32 {
        self.attached.get(&pane).copied().unwrap_or(0)
    }

    /// Records `writable_clients` attached to `pane`'s terminal.
    pub(super) fn set_attached(&mut self, pane: PaneId, writable_clients: u32) {
        if writable_clients == 0 {
            self.attached.remove(&pane);
        } else {
            self.attached.insert(pane, writable_clients);
        }
    }

    /// Holds `finished`, `pane`'s exited process, until its restart settles.
    /// Returns the handle it displaced, whose exit is no longer held.
    pub(super) fn hold_exit(
        &mut self,
        pane: PaneId,
        finished: Box<dyn ProcessHandle>,
    ) -> Option<Box<dyn ProcessHandle>> {
        self.held_exits.insert(pane, finished)
    }

    /// Takes `pane`'s held exit, to acknowledge or let go of.
    pub(super) fn release(&mut self, pane: PaneId) -> Option<Box<dyn ProcessHandle>> {
        self.held_exits.remove(&pane)
    }

    /// Every pane holding an exit.
    pub(super) fn holding(&self) -> Vec<PaneId> {
        self.held_exits.keys().copied().collect()
    }

    /// Lets go of every held exit but those of the panes `keep` names,
    /// unacknowledged, so the next service applies them again.
    pub(super) fn keep_held(&mut self, keep: impl Fn(PaneId) -> bool) {
        self.held_exits.retain(|pane, _| keep(*pane));
    }

    /// How many exits are held.
    #[cfg(test)]
    pub(super) fn held_count(&self) -> usize {
        self.held_exits.len()
    }
}
