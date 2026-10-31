use std::path::Path;

use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::json_dialect::JsonDialect;

/// Where in a JSON file an entry lives and how strictly the file is read.
#[derive(Clone, Debug, Getters, CopyGetters, TypedBuilder)]
pub(super) struct JsonPlace<'a> {
    /// The file, for messages.
    #[getset(get_copy = "pub(super)")]
    path: &'a Path,
    /// The keys of the objects leading to the table of servers, outermost
    /// first; the last is the table itself.
    #[getset(get = "pub(super)")]
    table: Vec<String>,
    /// How strictly the file is read.
    #[getset(get_copy = "pub(super)")]
    dialect: JsonDialect,
    /// The keys leading to a list of server names the agent keeps turned
    /// off, outermost first, read only to warn and never edited.
    #[getset(get = "pub(super)")]
    off_list: Option<Vec<String>>,
}
