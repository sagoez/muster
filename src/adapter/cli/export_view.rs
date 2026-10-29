use std::path::PathBuf;

use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::domain::value::ProjectName;

/// What `muster export` wrote, as `--json` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct ExportView {
    /// The file written.
    #[getset(get = "pub")]
    path: PathBuf,
    /// How many processes it holds.
    #[getset(get_copy = "pub")]
    processes: usize,
    /// The project exported.
    #[serde(skip)]
    #[getset(get = "pub")]
    project: ProjectName,
}
