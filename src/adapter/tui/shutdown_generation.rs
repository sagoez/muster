use nutype::nutype;

/// Identity of one graceful shutdown request for a live pane. A newer stop or
/// restart makes escalation events from an earlier request stale.
#[nutype(derive(Debug, Clone, Copy, PartialEq, Eq))]
pub(super) struct ShutdownGeneration(u64);

impl ShutdownGeneration {
    /// The value before a pane's first graceful shutdown request.
    pub(super) fn initial() -> Self {
        Self::new(0)
    }

    /// Returns the next generation, wrapping only after the integer space is
    /// exhausted so release builds never panic on the bookkeeping increment.
    pub(super) fn next(self) -> Self {
        Self::new(self.into_inner().wrapping_add(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Advancing produces a distinct request identity.
    #[test]
    fn next_advances_the_generation() {
        assert_eq!(
            ShutdownGeneration::initial().next(),
            ShutdownGeneration::new(1)
        );
    }
}
