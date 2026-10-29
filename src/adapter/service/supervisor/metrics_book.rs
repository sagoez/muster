use std::collections::HashMap;

use crate::{
    adapter::service::{PortBlock, PortHolder, ProcessUsage},
    domain::{
        port::{ListeningSocket, PortOccupant, PortOccupants},
        value::PaneId,
    },
};

/// What the live processes were last sampled using and listening on, which
/// ports every supervised process holds, and the starts those ports blocked.
pub(crate) struct MetricsBook {
    /// Latest sampled resource usage of every running pane.
    usage: HashMap<PaneId, ProcessUsage>,
    /// The ports each live pane's process tree was last seen listening on.
    ports: HashMap<PaneId, Vec<ListeningSocket>>,
    /// Every port a supervised process of any project listens on, and who
    /// holds it, as the service last shared it.
    held_ports: HashMap<u16, PortHolder>,
    /// Each pane whose last start was refused for a held port, and why.
    blocks: HashMap<PaneId, PortBlock>,
    /// Finds who listens on a port a start needs, when no supervised
    /// process was seen on it.
    port_occupants: Option<Box<dyn PortOccupants>>,
}

impl MetricsBook {
    /// An empty book that finds a port's occupant through `port_occupants`,
    /// when given.
    pub(super) fn new(port_occupants: Option<Box<dyn PortOccupants>>) -> Self {
        Self {
            usage: HashMap::new(),
            ports: HashMap::new(),
            held_ports: HashMap::new(),
            blocks: HashMap::new(),
            port_occupants,
        }
    }

    /// Swaps in the lookup of who listens on a port, for a test to inject a
    /// fake.
    #[cfg(test)]
    pub(super) fn set_port_occupants(&mut self, occupants: Box<dyn PortOccupants>) {
        self.port_occupants = Some(occupants);
    }

    /// `pane`'s latest sampled usage.
    pub(super) fn usage(&self, pane: PaneId) -> Option<ProcessUsage> {
        self.usage.get(&pane).copied()
    }

    /// Whether usage is cached for any pane.
    pub(super) fn has_usage(&self) -> bool {
        !self.usage.is_empty()
    }

    /// Takes `updated` as every pane's usage. Returns whether it changed.
    pub(super) fn replace_usage(&mut self, updated: HashMap<PaneId, ProcessUsage>) -> bool {
        let changed = updated != self.usage;
        self.usage = updated;
        changed
    }

    /// Forgets `pane`'s usage, for a process that ended or was replaced.
    pub(super) fn clear_usage(&mut self, pane: PaneId) {
        self.usage.remove(&pane);
    }

    /// Caches `usage` for `pane`, for a test to arrange.
    #[cfg(test)]
    pub(super) fn set_usage(&mut self, pane: PaneId, usage: ProcessUsage) {
        self.usage.insert(pane, usage);
    }

    /// The ports `pane`'s process tree was last seen listening on.
    pub(super) fn ports(&self, pane: PaneId) -> Vec<ListeningSocket> {
        self.ports.get(&pane).cloned().unwrap_or_default()
    }

    /// Every live pane's ports, as last seen.
    pub(super) fn listening(&self) -> impl Iterator<Item = (PaneId, &[ListeningSocket])> {
        self.ports
            .iter()
            .map(|(pane, sockets)| (*pane, sockets.as_slice()))
    }

    /// Takes `ports` as every live pane's ports.
    pub(super) fn replace_ports(&mut self, ports: HashMap<PaneId, Vec<ListeningSocket>>) {
        self.ports = ports;
    }

    /// Forgets `pane`'s ports, for a run that ended.
    pub(super) fn clear_ports(&mut self, pane: PaneId) {
        self.ports.remove(&pane);
    }

    /// Takes `held`, every port a supervised process of any project listens
    /// on.
    pub(super) fn set_held_ports(&mut self, held: HashMap<u16, PortHolder>) {
        self.held_ports = held;
    }

    /// Who holds `port` among the supervised processes of every project.
    pub(super) fn holder_of(&self, port: u16) -> Option<&PortHolder> {
        self.held_ports.get(&port)
    }

    /// Who listens on `port` by the OS socket table, when it can be asked.
    pub(super) fn occupant(&mut self, port: u16) -> Option<PortOccupant> {
        self.port_occupants.as_mut()?.occupant(port)
    }

    /// Why `pane`'s last start was refused, while it stands.
    pub(super) fn block(&self, pane: PaneId) -> Option<&PortBlock> {
        self.blocks.get(&pane)
    }

    /// Records why `pane`'s start was refused.
    pub(super) fn set_block(&mut self, pane: PaneId, block: PortBlock) {
        self.blocks.insert(pane, block);
    }

    /// Clears `pane`'s refusal, for a start nothing blocks.
    pub(super) fn clear_block(&mut self, pane: PaneId) {
        self.blocks.remove(&pane);
    }

    /// Forgets `pane`'s usage and ports, for a pane retired from the
    /// workspace.
    pub(super) fn forget(&mut self, pane: PaneId) {
        self.usage.remove(&pane);
        self.ports.remove(&pane);
    }
}
