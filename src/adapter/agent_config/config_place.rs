use std::path::PathBuf;

use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::{config_format::ConfigFormat, disabling::Disabling};

/// Where one config lives and how it is edited.
#[derive(Clone, Debug, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub(super) struct ConfigPlace {
    /// The file, as the agent names it.
    #[getset(get = "pub(super)")]
    path: PathBuf,
    /// How it is written.
    #[getset(get = "pub(super)")]
    format: ConfigFormat,
    /// Whether its folder is made when missing.
    #[getset(get_copy = "pub(super)")]
    makes_folder: bool,
    /// Whether Claude Code's lock guards it.
    #[getset(get_copy = "pub(super)")]
    locked: bool,
    /// The project folder a file shared with the project must stay inside.
    #[getset(get = "pub(super)")]
    confined_to: Option<PathBuf>,
    /// How the agent turns an entry off.
    #[getset(get_copy = "pub(super)")]
    disabling: Disabling,
}
