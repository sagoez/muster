use std::path::PathBuf;

use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::json_dialect::JsonDialect;

/// A list of server names an agent keeps turned off, outside the entries
/// themselves, as Claude Code keeps them.
#[derive(Clone, Debug, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub(super) struct DisabledList {
    /// The file holding it, as the agent names it.
    #[getset(get = "pub(super)")]
    path: PathBuf,
    /// The keys leading to the list, outermost first; the last is the list
    /// itself.
    #[getset(get = "pub(super)")]
    keys: Vec<String>,
    /// How strictly the file is read.
    #[getset(get_copy = "pub(super)")]
    dialect: JsonDialect,
    /// The project folder a file shared with the project must stay inside.
    #[getset(get = "pub(super)")]
    confined_to: Option<PathBuf>,
    /// Whether Claude Code's lock guards the file.
    #[getset(get_copy = "pub(super)")]
    locked: bool,
}
