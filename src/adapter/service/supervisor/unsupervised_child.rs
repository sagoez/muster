use std::{io, time::Instant};

use getset::CopyGetters;
use typed_builder::TypedBuilder;

use crate::{
    adapter::process_identity::LocalProcessIdentity,
    domain::value::{Pid, StartToken},
};

/// A child left running under a holder the service cannot reach: its pid and
/// the start token that tells it apart from a later process reusing the pid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, CopyGetters, TypedBuilder)]
pub(crate) struct UnsupervisedChild {
    /// The child's process id, which also names its process group.
    #[getset(get_copy = "pub(crate)")]
    pid: u32,
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
    /// token that cannot be read is no proof of one, so the child is kept.
    pub(crate) fn observe_expected(pid: u32, expected: Option<StartToken>) -> Option<Self> {
        if !pid_exists(pid) {
            return None;
        }
        let current = start_token_of(pid);
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

    /// Whether this same child still runs: its pid exists and, when a start
    /// token was recorded and one can be read now, the two match. A token that
    /// cannot be read counts as still running, so a transient read failure
    /// never lets a second copy start.
    pub(crate) fn is_running(&self) -> bool {
        pid_exists(self.pid)
            && LocalProcessIdentity::same_start(self.start_token, start_token_of(self.pid))
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

/// The start token of the process `pid`, when the platform reports one.
fn start_token_of(pid: u32) -> Option<StartToken> {
    Pid::try_new(pid)
        .ok()
        .and_then(LocalProcessIdentity::start_token)
}

/// The pid of init, which is never a holder's child. Its group id, negated,
/// would signal every process, so it and every lower pid count as absent.
const INIT_PID: libc::pid_t = 1;

/// Whether a process with `pid` exists. A pid at or below [`INIT_PID`] never
/// exists here, so it is never signalled; a process this user may not signal (`EPERM`) still
/// exists, so uncertainty never reports a running child gone.
fn pid_exists(pid: u32) -> bool {
    let Ok(raw) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if raw <= INIT_PID {
        return false;
    }
    // SAFETY: signal 0 only checks the pid; no signal is delivered.
    if unsafe { libc::kill(raw, 0) } == 0 {
        return true;
    }
    io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(test)]
mod tests {
    use std::process;

    use super::*;

    /// The test process itself is running and keeps its own start token.
    #[test]
    fn the_running_test_process_is_observed_and_still_running() {
        let child = UnsupervisedChild::observe_expected(process::id(), None).unwrap();
        assert!(child.is_running());
    }

    /// Pids zero and one are never observed, so their negated group ids, which
    /// reach far more than one child, are never signalled.
    #[test]
    fn pids_zero_and_one_are_never_observed() {
        assert!(UnsupervisedChild::observe_expected(0, None).is_none());
        assert!(UnsupervisedChild::observe_expected(1, None).is_none());
    }

    /// A different start token means the pid was reused by another process.
    #[test]
    fn a_reused_pid_is_not_the_same_child() {
        let child = UnsupervisedChild::builder()
            .pid(process::id())
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
        assert!(UnsupervisedChild::observe_expected(process::id(), foreign).is_none());

        let own = start_token_of(process::id());
        assert!(UnsupervisedChild::observe_expected(process::id(), own).is_some());
    }
}
