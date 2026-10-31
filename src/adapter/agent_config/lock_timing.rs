use std::time::Duration;

use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

/// How long a lock folder may go untouched before it counts as left by a
/// writer that died: `proper-lockfile`'s default, which Claude Code keeps.
const STALE_AFTER: Duration = Duration::from_secs(10);
/// How often muster touches a lock it holds, well within [`STALE_AFTER`]
/// so a long hold is never taken for a dead one.
const REFRESH_EVERY: Duration = Duration::from_secs(2);
/// The pauses between tries at a held lock, Claude Code's own: together
/// longer than a lock takes to go stale.
const RETRY_PAUSES: [Duration; 6] = [
    Duration::from_millis(200),
    Duration::from_millis(400),
    Duration::from_millis(800),
    Duration::from_millis(1600),
    Duration::from_millis(3200),
    Duration::from_millis(4000),
];

/// How Claude Code's config lock is waited for, judged stale and kept
/// fresh.
#[derive(Clone, Debug, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub(super) struct LockTiming {
    /// How long a lock may go untouched before it is taken over.
    #[getset(get_copy = "pub(super)")]
    stale_after: Duration,
    /// How often a lock held is touched.
    #[getset(get_copy = "pub(super)")]
    refresh_every: Duration,
    /// The pauses between tries at a held lock.
    #[getset(get = "pub(super)")]
    retry_pauses: Vec<Duration>,
}

impl LockTiming {
    /// The timing Claude Code's own `proper-lockfile` lock keeps.
    pub(super) fn claude_code() -> Self {
        Self::builder()
            .stale_after(STALE_AFTER)
            .refresh_every(REFRESH_EVERY)
            .retry_pauses(RETRY_PAUSES.to_vec())
            .build()
    }
}
