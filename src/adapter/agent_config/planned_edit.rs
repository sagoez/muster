use std::path::PathBuf;

use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::{config_target::ConfigTarget, edit_outcome::EditOutcome, file_edit::FileEdit};

/// An edit of one agent config, worked out against the file as read and
/// not yet written, so every edit of a command is checked before any is
/// made.
#[derive(Clone, Debug, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct PlannedEdit {
    /// The config edited.
    #[getset(get = "pub")]
    target: ConfigTarget,
    /// The config's file, as the agent names it.
    #[getset(get = "pub")]
    path: PathBuf,
    /// What the edit does.
    #[getset(get_copy = "pub")]
    outcome: EditOutcome,
    /// The file the edit writes, none when it writes nothing.
    #[getset(get = "pub(super)")]
    file: Option<FileEdit>,
    /// What the agent does with the entry that a person should know.
    #[getset(get_copy = "pub")]
    note: Option<&'static str>,
}
