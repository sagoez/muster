use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

use crate::{
    floor::{LogReading, LogStat},
    live::{hub_event::HubEvent, hub_handle::HubHandle, pause::Pause},
};

/// How often the running processes' logs are statted.
pub const LOGS_EVERY: Duration = Duration::from_secs(1);

/// The stat of the log at `path`, none when it cannot be statted, as when
/// the process has written none yet.
pub fn stat_log(path: &Path) -> Option<LogStat> {
    let metadata = fs::metadata(path).ok()?;
    Some(
        LogStat::builder()
            .size(metadata.len())
            .modified(metadata.modified().ok())
            .build(),
    )
}

/// Every [`LOGS_EVERY`], asks `hub` which logs to stat, stats each where
/// the holders' layout puts it and sends the stats back, even with none to
/// stat, so speaking ends on time; until the hub is gone.
pub fn poll_logs(hub: &HubHandle, pause: &Pause) {
    while pause.wait(LOGS_EVERY) {
        let Ok(watches) = hub.log_watches() else {
            return;
        };
        let readings = watches
            .iter()
            .map(|watch| {
                LogReading::builder()
                    .root(watch.root().clone())
                    .name(watch.name().clone())
                    .stat(stat_log(watch.path()))
                    .build()
            })
            .collect();
        if !hub.send(HubEvent::Logs {
            now: Instant::now(),
            readings,
        }) {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use muster_os::test_scratch::short_scratch;

    use super::*;

    /// A log's stat carries its size; a missing one has none.
    #[test]
    fn a_log_stats_its_size_and_a_missing_one_nothing() {
        let scratch = short_scratch("mv-");
        let log = scratch.path().join("web.log");
        fs::write(&log, b"ready\n").unwrap();

        assert_eq!(stat_log(&log).map(|stat| stat.size()), Some(6));
        assert_eq!(stat_log(&scratch.path().join("none.log")), None);
    }
}
