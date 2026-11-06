use std::mem;

/// Lines of a process's output dropped because rendering them panicked, as
/// one source of them counts them: reported the first time any are in a
/// run, and not again until the next run begins.
#[derive(Debug, Default)]
pub struct FaultBudget {
    /// Lines counted and not yet taken.
    pending: usize,
    /// Whether this run's lines were reported already.
    reported: bool,
}

impl FaultBudget {
    /// Counts `dropped` more lines.
    pub fn note(&mut self, dropped: usize) {
        self.pending += dropped;
    }

    /// The lines counted since the last call, the first time any are in
    /// the run; none after that until [`Self::renew`].
    pub fn take_report(&mut self) -> Option<usize> {
        let dropped = mem::take(&mut self.pending);
        if dropped == 0 || self.reported {
            return None;
        }
        self.reported = true;
        Some(dropped)
    }

    /// Begins a new run, whose dropped lines are reported afresh.
    pub fn renew(&mut self) {
        self.reported = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run is reported once, with what was counted by then, and the next
    /// run again.
    #[test]
    fn a_run_is_reported_once() {
        let mut budget = FaultBudget::default();
        assert_eq!(budget.take_report(), None);

        budget.note(2);
        budget.note(1);
        assert_eq!(budget.take_report(), Some(3));
        budget.note(4);
        assert_eq!(budget.take_report(), None);

        budget.renew();
        budget.note(1);
        assert_eq!(budget.take_report(), Some(1));
    }
}
