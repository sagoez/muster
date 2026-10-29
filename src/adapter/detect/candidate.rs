use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::detect_source::DetectSource;
use crate::domain::value::{CommandLine, ProcessName};

/// A process `muster init` offers to add, read from a file in the folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct Candidate {
    /// The name it would be added under.
    #[getset(get = "pub")]
    name: ProcessName,
    /// The command it would run.
    #[getset(get = "pub")]
    command: CommandLine,
    /// The file it was found in.
    #[getset(get_copy = "pub")]
    source: DetectSource,
    /// Whether it looks like a dev process worth starting, so it is chosen
    /// unless the person says otherwise.
    #[getset(get_copy = "pub")]
    suggested: bool,
}

impl Candidate {
    /// The candidate renamed `name`, after a collision with another source.
    pub fn with_name(mut self, name: ProcessName) -> Self {
        self.name = name;
        self
    }
}
