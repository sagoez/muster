use std::path::PathBuf;

use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::{pane_command::PaneCommand, pane_focus::PaneFocus, split_direction::SplitDirection};

/// A pane to open beside the caller's: where it starts, what it runs,
/// whether it takes the focus and which way it splits.
#[derive(Clone, Debug, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct PaneRequest {
    /// The folder the pane starts in.
    #[getset(get = "pub")]
    cwd: PathBuf,
    /// What the pane runs.
    #[getset(get = "pub")]
    command: PaneCommand,
    /// Whether the pane takes the user's focus.
    #[getset(get_copy = "pub")]
    focus: PaneFocus,
    /// Which way to split, or none for the workspace to pick from the
    /// caller's pane.
    #[getset(get_copy = "pub")]
    direction: Option<SplitDirection>,
}
