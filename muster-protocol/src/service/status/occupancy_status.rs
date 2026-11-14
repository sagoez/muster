use std::time::Duration;

use getset::CopyGetters;
use muster_domain::value::IdleStop;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

/// Who is in a project and when it stops for want of anyone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
#[serde(rename_all = "kebab-case")]
pub struct OccupancyStatus {
    /// Live shells whose folder is in the project.
    shells: u32,
    /// How long the project keeps running with no shell in it: its own
    /// override, else the setting.
    #[serde(with = "crate::wire::form")]
    idle_stop: IdleStop,
    /// How long until the pending idle stop, none when none is pending.
    #[serde(with = "humantime_serde")]
    stops_in: Option<Duration>,
}
