use nutype::nutype;

/// Identity of one directory-completion request. A later edit advances the
/// generation so an older filesystem result cannot replace current candidates.
#[nutype(derive(Debug, Clone, Copy, PartialEq, Eq))]
pub(super) struct CompletionGeneration(u64);

impl CompletionGeneration {
    /// The value before the first asynchronous completion request.
    pub(super) fn initial() -> Self {
        Self::new(0)
    }

    /// Returns the next request generation, wrapping only after the integer
    /// space is exhausted.
    pub(super) fn next(self) -> Self {
        Self::new(self.into_inner().wrapping_add(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_advances_the_generation() {
        assert_eq!(
            CompletionGeneration::initial().next(),
            CompletionGeneration::new(1)
        );
    }
}
