use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

/// Most connections served at once; the server closes one more at accept.
pub const MAX_CONNECTIONS: usize = 32;

/// One of the [`MAX_CONNECTIONS`] places a connection takes while it is
/// served, given back when dropped.
pub struct ConnectionSlot {
    /// How many places are taken, this one included.
    taken: Arc<AtomicUsize>,
}

impl ConnectionSlot {
    /// A place counted in `taken`, or none when all are taken.
    pub fn take(taken: &Arc<AtomicUsize>) -> Option<Self> {
        if taken.fetch_add(1, Ordering::AcqRel) >= MAX_CONNECTIONS {
            taken.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        Some(Self {
            taken: Arc::clone(taken),
        })
    }
}

impl Drop for ConnectionSlot {
    /// Gives the place back.
    fn drop(&mut self) {
        self.taken.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Places run out at the cap and come back when dropped.
    #[test]
    fn places_run_out_at_the_cap_and_come_back() {
        let taken = Arc::new(AtomicUsize::new(0));

        let slots = (0..MAX_CONNECTIONS)
            .map(|_| ConnectionSlot::take(&taken).unwrap())
            .collect::<Vec<_>>();

        assert!(ConnectionSlot::take(&taken).is_none());
        drop(slots);
        assert_eq!(taken.load(Ordering::Acquire), 0);
        assert!(ConnectionSlot::take(&taken).is_some());
    }
}
