use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::split_direction::SplitDirection;
use crate::domain::value::WorkspacePaneId;

/// A pane a terminal workspace opened, and the way it split.
#[derive(Clone, Debug, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct OpenedPane {
    /// The new pane.
    #[getset(get = "pub")]
    pane: WorkspacePaneId,
    /// Which side of the caller's pane it opened on.
    #[getset(get_copy = "pub")]
    direction: SplitDirection,
}
