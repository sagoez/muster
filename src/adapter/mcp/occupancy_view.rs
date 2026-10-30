use getset::CopyGetters;
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::{
    adapter::{client::whole_seconds_up, service::OccupancyStatus},
    domain::value::IdleStop,
};

/// Who is in a project and when it stops for want of anyone, as
/// `stack_status` shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, CopyGetters, TypedBuilder)]
pub struct OccupancyView {
    /// Live shells whose folder is in the project.
    #[getset(get_copy = "pub")]
    shells: u32,
    /// How long the project keeps running with no shell in it: whole seconds
    /// in JSON, or null for never.
    #[serde(with = "crate::adapter::client::idle_stop_seconds")]
    #[getset(get_copy = "pub")]
    idle_stop: IdleStop,
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
            idle_stop: status.idle_stop(),
            stops_in_s: status.stops_in().map(whole_seconds_up),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::domain::value::PositiveDuration;

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
            serde_json::to_value(pending).unwrap(),
            serde_json::json!({ "shells": 0, "idle_stop": 1800, "stops_in_s": 1734 })
        );
        assert_eq!(
            serde_json::to_value(occupied).unwrap(),
            serde_json::json!({ "shells": 2, "idle_stop": 1800 })
        );
    }

    /// An idle stop that never comes is null, and the view keeps it typed.
    #[test]
    fn a_never_idle_stop_is_null() {
        let never = OccupancyStatus::builder()
            .shells(1)
            .idle_stop(IdleStop::Never)
            .stops_in(None)
            .build();
        let view = OccupancyView::from(&never);

        assert_eq!(view.idle_stop(), IdleStop::Never);
        assert_eq!(
            serde_json::to_value(view).unwrap(),
            serde_json::json!({ "shells": 1, "idle_stop": null })
        );
    }
}
