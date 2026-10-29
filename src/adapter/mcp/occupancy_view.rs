use std::time::Duration;

use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::adapter::service::OccupancyStatus;

/// Who is in a project and when it stops for want of anyone, as
/// `stack_status` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct OccupancyView {
    /// Live shells whose folder is in the project.
    #[getset(get_copy = "pub")]
    shells: u32,
    /// How long the project keeps running with no shell in it, such as `30m`
    /// or `never`.
    #[getset(get = "pub")]
    idle_stop: String,
    /// Whole seconds, rounded up, until the pending idle stop; left out when
    /// none is pending.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get_copy = "pub")]
    stops_in_s: Option<u64>,
}

impl From<&OccupancyStatus> for OccupancyView {
    fn from(status: &OccupancyStatus) -> Self {
        Self {
            shells: status.shells(),
            idle_stop: status.idle_stop().to_string(),
            stops_in_s: status.stops_in().map(whole_seconds_up),
        }
    }
}

/// `duration` in whole seconds, a started second counted, so a pending stop
/// never reads as zero.
fn whole_seconds_up(duration: Duration) -> u64 {
    duration
        .as_secs()
        .saturating_add(u64::from(duration.subsec_nanos() > 0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::value::{IdleStop, PositiveDuration};

    /// The status of a project with `shells` in it and a stop `stops_in`
    /// away under a 30 minute idle stop.
    fn status(shells: u32, stops_in: Option<Duration>) -> OccupancyStatus {
        OccupancyStatus::builder()
            .shells(shells)
            .idle_stop(IdleStop::After(
                PositiveDuration::try_new(Duration::from_secs(1_800)).unwrap(),
            ))
            .stops_in(stops_in)
            .build()
    }

    /// A pending stop reads in whole seconds rounded up, and none is left
    /// out of the JSON.
    #[test]
    fn stops_in_rounds_up_and_is_left_out_without_a_timer() {
        let pending = OccupancyView::from(&status(0, Some(Duration::from_millis(1_733_200))));
        let occupied = OccupancyView::from(&status(2, None));

        assert_eq!(
            serde_json::to_value(&pending).unwrap(),
            serde_json::json!({ "shells": 0, "idle_stop": "30m", "stops_in_s": 1734 })
        );
        assert_eq!(
            serde_json::to_value(&occupied).unwrap(),
            serde_json::json!({ "shells": 2, "idle_stop": "30m" })
        );
        assert_eq!(whole_seconds_up(Duration::from_secs(5)), 5);
        assert_eq!(whole_seconds_up(Duration::from_nanos(1)), 1);
    }
}
