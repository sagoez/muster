use std::path::PathBuf;

use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::{adapter::client::ActionReport, domain::value::ProjectName};

/// What `projects rm` did, as `--json` shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct ProjectRemoval {
    /// The project's name, or none when its stored row did not read.
    #[getset(get = "pub")]
    name: Option<ProjectName>,
    /// Its folder.
    #[getset(get = "pub")]
    root: PathBuf,
    /// Whether it was still registered when removed.
    #[getset(get_copy = "pub")]
    removed: bool,
    /// What became of its processes as they were stopped, or none when its
    /// stored row or processes did not read and the service could not stop
    /// them first.
    #[getset(get = "pub")]
    stopped: Option<ActionReport>,
}

impl ProjectRemoval {
    /// Whether a process it stopped did not end up stopped.
    pub fn failed(&self) -> bool {
        self.stopped.as_ref().is_some_and(ActionReport::failed)
    }
}
