use std::collections::{BTreeSet, btree_set::Iter};

use getset::Getters;

use super::too_many_ports::TooManyPorts;
use crate::domain::value::LocalPort;

/// Most ports one process may declare, since every launch checks each of
/// them before it starts.
pub const DECLARED_PORTS_MAX: usize = 16;

/// The TCP ports a process declares it listens on, ascending, at most
/// [`DECLARED_PORTS_MAX`] of them.
/// No builder: every way in is checked against the bound.
#[derive(Clone, Debug, Default, PartialEq, Eq, Getters)]
#[getset(get = "pub")]
pub struct DeclaredPorts {
    /// The ports, each once.
    ports: BTreeSet<LocalPort>,
}

impl DeclaredPorts {
    /// The set holding `ports`, a repeated port counted once.
    ///
    /// # Errors
    /// Returns [`TooManyPorts`] when they name more than
    /// [`DECLARED_PORTS_MAX`] distinct ports.
    pub fn of(ports: impl IntoIterator<Item = LocalPort>) -> Result<Self, TooManyPorts> {
        let ports: BTreeSet<LocalPort> = ports.into_iter().collect();
        if ports.len() > DECLARED_PORTS_MAX {
            return Err(TooManyPorts::builder().found(ports.len()).build());
        }
        Ok(Self { ports })
    }

    /// The ports in ascending order.
    pub fn iter(&self) -> Iter<'_, LocalPort> {
        self.ports.iter()
    }

    /// Whether it declares no port.
    pub fn is_empty(&self) -> bool {
        self.ports.is_empty()
    }
}

impl<'a> IntoIterator for &'a DeclaredPorts {
    type IntoIter = Iter<'a, LocalPort>;
    type Item = &'a LocalPort;

    /// The ports in ascending order.
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ports `1..=count`.
    fn ports(count: usize) -> Vec<LocalPort> {
        (1..=count)
            .map(|port| LocalPort::try_new(u16::try_from(port).unwrap()).unwrap())
            .collect()
    }

    /// Up to the maximum ports are kept, ascending and each once; one more
    /// is refused naming the limit.
    #[test]
    fn the_port_count_is_bounded() {
        let full = DeclaredPorts::of(ports(DECLARED_PORTS_MAX)).unwrap();
        assert_eq!(full.iter().count(), DECLARED_PORTS_MAX);

        let refused = DeclaredPorts::of(ports(DECLARED_PORTS_MAX + 1)).unwrap_err();
        assert_eq!(
            refused,
            TooManyPorts::builder()
                .found(DECLARED_PORTS_MAX + 1)
                .build()
        );
        assert_eq!(
            refused.to_string(),
            format!(
                "{} ports are more than the {DECLARED_PORTS_MAX} a process may declare",
                DECLARED_PORTS_MAX + 1
            )
        );
    }

    /// A repeated port is one port, and the set reads back ascending.
    #[test]
    fn a_repeated_port_counts_once_and_the_set_is_ascending() {
        let mut repeated = ports(DECLARED_PORTS_MAX);
        repeated.reverse();
        repeated.extend(ports(1));

        let declared = DeclaredPorts::of(repeated).unwrap();

        assert_eq!(
            declared.iter().copied().collect::<Vec<_>>(),
            ports(DECLARED_PORTS_MAX)
        );
        assert!(DeclaredPorts::default().is_empty());
        assert!(!declared.is_empty());
    }
}
