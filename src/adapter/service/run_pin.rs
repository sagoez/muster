use super::supervisor::SpawnGeneration;

/// Which runs of a process a wait is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunPin {
    /// The run current at `from` and every later one.
    From(SpawnGeneration),
    /// Only runs started after `after`, so a start or restart never sees the
    /// run it replaced.
    After(SpawnGeneration),
}

impl RunPin {
    /// Whether the run `generation` is one the wait is about.
    pub fn counts(self, generation: SpawnGeneration) -> bool {
        match self {
            Self::From(from) => generation >= from,
            Self::After(after) => generation > after,
        }
    }

    /// Whether the wait is only about a run its own request starts.
    pub fn is_fresh(self) -> bool {
        matches!(self, Self::After(_))
    }
}
