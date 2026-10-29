use getset::CopyGetters;
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::process_view::whole_seconds_up;
use crate::{adapter::service::OccupancyStatus, domain::value::IdleStop};

/// Who is in a project and when it stops for want of anyone, as `--json`
/// shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct OccupancyView {
    /// Live shells whose folder is in the project.
    shells: u32,
    /// How long it keeps running with no shell in it, such as `30m` or
    /// `never`.
    #[serde(with = "crate::adapter::wire::form")]
    idle_stop: IdleStop,
    /// Whole seconds, rounded up, until the pending idle stop; none when
    /// none is pending.
    stops_in: Option<u64>,
}

impl From<&OccupancyStatus> for OccupancyView {
    fn from(status: &OccupancyStatus) -> Self {
        Self {
            shells: status.shells(),
            idle_stop: status.idle_stop(),
            stops_in: status.stops_in().map(whole_seconds_up),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::domain::value::PositiveDuration;

    /// The occupancy of a project left `stops_in` before its idle stop.
    fn left(stops_in: Option<Duration>) -> OccupancyStatus {
        OccupancyStatus::builder()
            .shells(0)
            .idle_stop(IdleStop::After(
                PositiveDuration::try_new(Duration::from_secs(1800)).unwrap(),
            ))
            .stops_in(stops_in)
            .build()
    }

    /// Any part of a second left counts as one, so a stop under a second
    /// away never reads as 0.
    #[test]
    fn stops_in_rounds_up_to_whole_seconds() {
        let view = |millis| OccupancyView::from(&left(Some(Duration::from_millis(millis))));

        assert_eq!(view(1_734_000).stops_in(), Some(1734));
        assert_eq!(view(1_733_001).stops_in(), Some(1734));
        assert_eq!(view(1).stops_in(), Some(1));
        assert_eq!(OccupancyView::from(&left(None)).stops_in(), None);
        assert_eq!(
            serde_json::to_value(view(1_733_400)).unwrap(),
            json!({"shells": 0, "idle_stop": "30m", "stops_in": 1734})
        );
    }
}
