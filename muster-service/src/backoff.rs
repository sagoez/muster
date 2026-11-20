use std::time::Duration;

use typed_builder::TypedBuilder;

/// A wait that doubles each time it is taken, up to a cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, TypedBuilder)]
pub(crate) struct Backoff {
    /// The wait the next step takes.
    wait: Duration,
    /// The longest any wait grows to.
    cap: Duration,
}

impl Backoff {
    /// The wait to take now, doubling the next one up to the cap.
    pub(crate) fn step(&mut self) -> Duration {
        let now = self.wait;
        self.wait = self.wait.saturating_mul(2).min(self.cap);
        now
    }

    /// The wait after `steps` have been taken, leaving this backoff as is.
    pub(crate) fn after(self, steps: u32) -> Duration {
        let mut backoff = self;
        for _ in 0..steps {
            if backoff.wait == backoff.cap {
                break;
            }
            backoff.step();
        }
        backoff.wait
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST: Duration = Duration::from_millis(200);
    const CAP: Duration = Duration::from_secs(1);

    /// A backoff from [`FIRST`] up to [`CAP`].
    fn backoff() -> Backoff {
        Backoff::builder().wait(FIRST).cap(CAP).build()
    }

    #[test]
    fn each_step_doubles_the_next_up_to_the_cap() {
        let mut backoff = backoff();

        let waits: Vec<Duration> = (0..5).map(|_| backoff.step()).collect();

        assert_eq!(
            waits,
            [200, 400, 800, 1000, 1000].map(Duration::from_millis)
        );
    }

    #[test]
    fn after_steps_is_the_wait_that_many_steps_reach() {
        let backoff = backoff();

        assert_eq!(backoff.after(0), FIRST);
        assert_eq!(backoff.after(2), Duration::from_millis(800));
        assert_eq!(backoff.after(u32::MAX), CAP);
    }
}
