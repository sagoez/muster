use nutype::nutype;

/// Identity of one spawn attempt for a pane. Events from earlier generations
/// are stale after a restart, stop cancellation, or project switch.
#[nutype(derive(Debug, Clone, Copy, PartialEq, Eq))]
pub(super) struct SpawnGeneration(u64);

impl SpawnGeneration {
    /// The value before a pane's first spawn attempt.
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

    #[test]
    fn next_advances_the_generation() {
        assert_eq!(SpawnGeneration::initial().next(), SpawnGeneration::new(1));
    }
}
