use std::time::Instant;

use getset::CopyGetters;
use typed_builder::TypedBuilder;

use crate::{
    adapter::{process_identity::LocalProcessIdentity, signal_target::SignalTarget},
    domain::value::{Pid, StartToken},
};

/// A child left running under a holder the service cannot reach: its pid and
/// the start token that tells it apart from a later process reusing the pid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, CopyGetters, TypedBuilder)]
pub(crate) struct UnsupervisedChild {
    /// The child's process id, which also names its process group.
    #[getset(get_copy = "pub(crate)")]
    pid: Pid,
    /// The child's start token when the platform reports one.
    start_token: Option<StartToken>,
    /// Kills sent since the last graceful stop, bounding the wait for a child
    /// that will not die.
    #[builder(default)]
    #[getset(get_copy = "pub(crate)")]
    kill_polls: u32,
    /// When the grace of a graceful stop in progress ends, until which the
    /// child is only watched for its exit.
    #[builder(default)]
    #[getset(get_copy = "pub(crate)")]
    grace_deadline: Option<Instant>,
}

impl UnsupervisedChild {
    /// The child running as `pid` that was started with `expected`, when that
    /// child still runs, recorded with its start token. A pid now held by a
    /// process with another start token is a stranger and is never recorded; a
    /// token that cannot be read is no proof of one, so the child is kept. A
    /// child already dead and waiting to be reaped, as a holder's child is for
    /// a moment after both were killed, is gone and never recorded.
    pub(crate) fn observe_expected(pid: Pid, expected: Option<StartToken>) -> Option<Self> {
        if !SignalTarget::Process(pid).is_alive() || LocalProcessIdentity::has_exited(pid) {
            return None;
        }
        let current = LocalProcessIdentity::start_token(pid);
        if !LocalProcessIdentity::same_start(expected, current) {
            return None;
        }
        Some(
            Self::builder()
                .pid(pid)
                .start_token(expected.or(current))
                .build(),
        )
    }

    /// Whether this same child still runs: its pid exists, is not dead and
    /// waiting to be reaped, as the child of a stopped holder stays once
    /// killed, and, when a start token was recorded and one can be read now,
    /// the two match. A token that cannot be read counts as still running, so
    /// a transient read failure never lets a second copy start.
    pub(crate) fn is_running(&self) -> bool {
        LocalProcessIdentity::is_same_process(self.pid, self.start_token)
            && !LocalProcessIdentity::has_exited(self.pid)
    }

    /// Counts one more kill sent while waiting for the child to die, and
    /// returns the new count.
    pub(crate) fn count_kill(&mut self) -> u32 {
        self.kill_polls += 1;
        self.kill_polls
    }

    /// Starts a stop afresh: no kills counted yet, and the grace a graceful
    /// stop gives the child until `grace_deadline`, `None` for none.
    pub(crate) fn begin_stop(&mut self, grace_deadline: Option<Instant>) {
        self.kill_polls = 0;
        self.grace_deadline = grace_deadline;
    }

    /// Ends the grace of a graceful stop, so the next check kills.
    pub(crate) fn end_grace(&mut self) {
        self.grace_deadline = None;
    }
}

#[cfg(test)]
mod tests {
    use std::process;
    #[cfg(target_os = "linux")]
    use std::{process::Command, thread, time::Duration};

    use super::*;

    /// How long the test child would sleep, far past the test.
    #[cfg(target_os = "linux")]
    const CHILD_SLEEP: &str = "600";
    /// Bound on waiting for a killed child to die.
    #[cfg(target_os = "linux")]
    const DEATH_WAIT: Duration = Duration::from_secs(5);
    /// Interval between checks that a killed child died.
    #[cfg(target_os = "linux")]
    const DEATH_POLL: Duration = Duration::from_millis(5);

    /// This test process.s pid.
    fn own_pid() -> Pid {
        Pid::try_new(process::id()).unwrap()
    }

    /// The test process itself is running and keeps its own start token.
    #[test]
    fn the_running_test_process_is_observed_and_still_running() {
        let child = UnsupervisedChild::observe_expected(own_pid(), None).unwrap();
        assert!(child.is_running());
    }

    /// A child that died but was not yet reaped, as a holder's child is for a
    /// moment after both were killed, is not left unsupervised, and one
    /// already observed no longer runs once it dies.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_child_that_died_unreaped_is_not_observed() {
        let mut child = Command::new("sleep").arg(CHILD_SLEEP).spawn().unwrap();
        let observed =
            UnsupervisedChild::observe_expected(Pid::try_new(child.id()).unwrap(), None).unwrap();
        child.kill().unwrap();
        let pid = Pid::try_new(child.id()).unwrap();
        let started = Instant::now();
        while !LocalProcessIdentity::has_exited(pid) {
            assert!(started.elapsed() < DEATH_WAIT, "the child never died");
            thread::sleep(DEATH_POLL);
        }

        assert!(
            SignalTarget::Process(pid).is_alive(),
            "the child is not reaped yet"
        );
        assert!(
            UnsupervisedChild::observe_expected(Pid::try_new(child.id()).unwrap(), None).is_none()
        );
        assert!(!observed.is_running(), "a dead child no longer runs");
        child.wait().unwrap();
    }

    /// A different start token means the pid was reused by another process.
    #[test]
    fn a_reused_pid_is_not_the_same_child() {
        let child = UnsupervisedChild::builder()
            .pid(own_pid())
            .start_token(Some(StartToken::try_new(1).unwrap()))
            .build();
        assert!(!child.is_running());
    }

    /// Tokens are compared only when both are known: a recorded token whose
    /// process cannot be read now is still the same child.
    #[test]
    fn an_unreadable_start_token_counts_as_the_same_child() {
        let token = Some(StartToken::try_new(7).unwrap());
        let other = Some(StartToken::try_new(8).unwrap());

        assert!(LocalProcessIdentity::same_start(token, None));
        assert!(LocalProcessIdentity::same_start(None, token));
        assert!(LocalProcessIdentity::same_start(token, token));
        assert!(!LocalProcessIdentity::same_start(token, other));
    }

    /// A holder record's child whose pid now belongs to another process is
    /// never recorded as the orphan.
    #[test]
    fn an_expected_child_whose_pid_was_reused_is_not_observed() {
        let foreign = Some(StartToken::try_new(1).unwrap());
        assert!(UnsupervisedChild::observe_expected(own_pid(), foreign).is_none());

        let own = LocalProcessIdentity::start_token(own_pid());
        assert!(UnsupervisedChild::observe_expected(own_pid(), own).is_some());
    }
}
