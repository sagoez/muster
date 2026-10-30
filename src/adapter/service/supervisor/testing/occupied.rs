use std::{
    collections::BTreeSet,
    sync::{Arc, PoisonError},
};

use super::asked_scans::AskedScans;
use crate::domain::{
    port::{PortOccupant, PortOccupants},
    value::LocalPort,
};

/// Answers port lookups with one occupant, on every port or only on some,
/// recording each scan it was asked for.
pub(crate) struct Occupied {
    /// Who listens on the held ports, if anyone.
    occupant: Option<PortOccupant>,
    /// The ports held, or every port when none are named.
    only: Option<BTreeSet<LocalPort>>,
    /// The ports of every scan asked.
    asked: AskedScans,
}

impl Occupied {
    /// Every port held by `occupant`, or by nobody.
    pub(crate) fn by(occupant: Option<PortOccupant>) -> Self {
        Self {
            occupant,
            only: None,
            asked: AskedScans::default(),
        }
    }

    /// Only `ports` held, each by `occupant`.
    pub(crate) fn on(ports: &[LocalPort], occupant: PortOccupant) -> Self {
        Self {
            occupant: Some(occupant),
            only: Some(ports.iter().copied().collect()),
            asked: AskedScans::default(),
        }
    }

    /// The scans asked of this lookup, readable after it is handed to a
    /// supervisor.
    pub(crate) fn asked(&self) -> AskedScans {
        Arc::clone(&self.asked)
    }
}

impl PortOccupants for Occupied {
    /// The lowest of `ports` held, recording the scan.
    fn first_occupied(&mut self, ports: &[LocalPort]) -> Option<(LocalPort, PortOccupant)> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ports.to_vec());
        let occupant = self.occupant.clone()?;
        let port = ports
            .iter()
            .copied()
            .filter(|port| self.only.as_ref().is_none_or(|only| only.contains(port)))
            .min()?;
        Some((port, occupant))
    }
}
