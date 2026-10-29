use nutype::nutype;

/// Identity of one try at adopting a pane's holder. A result or timer of any
/// other try than the pane's current one is stale.
#[nutype(default = 0, derive(Debug, Clone, Copy, PartialEq, Eq, Default))]
pub struct RetryId(u64);

impl RetryId {
    /// The id after this one, wrapping only after the integer space is
    /// exhausted.
    pub fn next(self) -> Self {
        Self::new(self.into_inner().wrapping_add(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_advances_the_id() {
        assert_eq!(RetryId::default().next(), RetryId::new(1));
    }
}
