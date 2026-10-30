//! The process groups of the short helper calls muster waits on, such as one
//! herdr call, tracked so that whatever ends muster ends them too: an ending
//! signal or an MCP client closing stdio.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
#[cfg(test)]
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

use rustix::process::{Pid as RawPid, Signal, kill_process_group};

use crate::domain::value::Pid;

/// The most calls tracked at once; one past it is refused.
const MAX_GROUPS: usize = 64;
/// A slot no group holds.
const FREE: u32 = 0;

/// The leader of each tracked group, one per slot.
static GROUPS: [AtomicU32; MAX_GROUPS] = [const { AtomicU32::new(FREE) }; MAX_GROUPS];
/// Set once muster is ending, after which no group is tracked.
static ENDED: AtomicBool = AtomicBool::new(false);
/// How many fake herdrs live in the test process, whose calls a test
/// raising an ending signal would kill; that test holds it while it raises.
#[cfg(test)]
static LIVE_FAKES: Mutex<usize> = Mutex::new(0);
/// Signalled each time the last live fake herdr goes.
#[cfg(test)]
static NO_FAKES: Condvar = Condvar::new();

/// Counts a fake herdr in, waiting while a test that raises an ending
/// signal runs.
#[cfg(test)]
pub fn fake_lives() {
    *LIVE_FAKES.lock().unwrap_or_else(PoisonError::into_inner) += 1;
}

/// Counts a fake herdr out.
#[cfg(test)]
pub fn fake_gone() {
    let mut live = LIVE_FAKES.lock().unwrap_or_else(PoisonError::into_inner);
    *live = live.saturating_sub(1);
    if *live == 0 {
        NO_FAKES.notify_all();
    }
}

/// Waits until no fake herdr lives and keeps new ones from starting while
/// the returned guard lives, for a test that raises an ending signal and so
/// kills every group the test process tracks. Its own fake is made after.
#[cfg(test)]
pub fn sweep_alone() -> MutexGuard<'static, usize> {
    let live = LIVE_FAKES.lock().unwrap_or_else(PoisonError::into_inner);
    NO_FAKES
        .wait_while(live, |live| *live > 0)
        .unwrap_or_else(PoisonError::into_inner)
}

/// One call's process group, tracked until dropped. An RAII guard, so it
/// has a hand-written constructor.
#[derive(Debug)]
pub struct ChildGroup {
    /// The slot it holds.
    slot: usize,
}

impl ChildGroup {
    /// Tracks the group `leader` leads. None once muster is ending or while
    /// the most groups it tracks are; the caller then stops the group itself.
    pub fn track(leader: Pid) -> Option<Self> {
        if ENDED.load(Ordering::SeqCst) {
            return None;
        }
        let slot = GROUPS.iter().position(|slot| {
            slot.compare_exchange(
                FREE,
                leader.into_inner(),
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok()
        })?;
        let tracked = Self { slot };
        // An end that raced the claim may have swept the slots before it.
        if ENDED.load(Ordering::SeqCst) {
            return None;
        }
        Some(tracked)
    }
}

impl Drop for ChildGroup {
    /// Stops tracking the group.
    fn drop(&mut self) {
        GROUPS[self.slot].store(FREE, Ordering::SeqCst);
    }
}

/// Kills every tracked group. Async-signal-safe: it only loads atomics and
/// calls `kill`, so a signal handler may run it.
pub fn kill_all() {
    for slot in &GROUPS {
        let leader = slot.load(Ordering::SeqCst);
        if let Some(group) = i32::try_from(leader).ok().and_then(RawPid::from_raw) {
            let _ = kill_process_group(group, Signal::KILL);
        }
    }
}

/// Kills every tracked group and refuses to track another, for a muster
/// about to exit.
pub fn end_all() {
    ENDED.store(true, Ordering::SeqCst);
    kill_all();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tracked group holds its slot until it drops. The leader is past
    /// any real pid, so a parallel test's sweep never signals it.
    #[test]
    fn a_group_holds_its_slot_until_dropped() {
        let tracked = ChildGroup::track(Pid::try_new(u32::MAX).unwrap()).unwrap();
        let slot = tracked.slot;
        assert_eq!(GROUPS[slot].load(Ordering::SeqCst), u32::MAX);
        drop(tracked);
        assert_ne!(GROUPS[slot].load(Ordering::SeqCst), u32::MAX);
    }
}
