use std::time::Duration;

use nutype::nutype;

/// A moment as whole seconds since the Unix epoch: when coordination state was
/// written, or when a lock or a todo's claim lapses.
#[nutype(derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Display))]
pub struct EpochSeconds(u64);

impl EpochSeconds {
    /// The moment `span` after this one, held at the latest representable
    /// second rather than wrapping.
    pub fn after(self, span: Duration) -> Self {
        Self::new(self.into_inner().saturating_add(span.as_secs()))
    }

    /// How long from this moment until `later`, zero once `later` has passed.
    pub fn until(self, later: Self) -> Duration {
        Duration::from_secs(later.into_inner().saturating_sub(self.into_inner()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A span moves a moment forward, and the time until a past moment is
    /// zero.
    #[test]
    fn spans_between_moments_never_go_negative() {
        let start = EpochSeconds::new(100);
        let later = start.after(Duration::from_secs(30));

        assert_eq!(later, EpochSeconds::new(130));
        assert_eq!(start.until(later), Duration::from_secs(30));
        assert_eq!(later.until(start), Duration::ZERO);
        assert_eq!(
            EpochSeconds::new(u64::MAX).after(Duration::from_secs(1)),
            EpochSeconds::new(u64::MAX)
        );
    }
}
