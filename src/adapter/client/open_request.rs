use std::path::PathBuf;

use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use crate::domain::terminal_workspace::{PaneFocus, SplitDirection};

/// How a driver asks for a process to be opened in a pane beside its
/// caller's.
#[derive(Clone, Debug, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct OpenRequest {
    /// The muster binary the pane runs to attach.
    #[getset(get = "pub")]
    binary: PathBuf,
    /// Whether the pane only watches the process.
    #[getset(get_copy = "pub")]
    read_only: bool,
    /// Whether the pane takes the user's focus.
    #[getset(get_copy = "pub")]
    focus: PaneFocus,
    /// Which way to split, or none for the workspace to pick.
    #[getset(get_copy = "pub")]
    direction: Option<SplitDirection>,
}
