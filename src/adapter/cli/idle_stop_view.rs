use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::idle_stop_source::IdleStopSource;
use crate::{adapter::client::ProjectIdentityView, domain::value::IdleStop};

/// A project's idle stop after `muster projects set`, as `--json` prints it:
/// the value in effect and whether it is the project's own or the setting's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct IdleStopView {
    /// The project.
    #[getset(get = "pub")]
    project: ProjectIdentityView,
    /// The idle stop in effect: whole seconds in JSON, or null for never.
    #[serde(with = "crate::adapter::client::idle_stop_seconds")]
    #[getset(get_copy = "pub")]
    idle_stop: IdleStop,
    /// Where it comes from.
    #[getset(get_copy = "pub")]
    source: IdleStopSource,
}
