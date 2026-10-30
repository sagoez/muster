use std::{ffi::OsString, path::PathBuf};

use getset::Getters;
use typed_builder::TypedBuilder;

/// What a new pane runs, as a program and its arguments; never a shell
/// string, so no argument is ever read as shell syntax.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct PaneCommand {
    /// The program.
    program: PathBuf,
    /// Its arguments, in order.
    args: Vec<OsString>,
}
