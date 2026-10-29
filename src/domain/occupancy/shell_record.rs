use getset::Getters;
use typed_builder::TypedBuilder;

use super::{ReportStamp, ShellId};
use crate::domain::value::ProjectRoot;

/// A shell's report, as the service is told it and as it is kept between
/// services: which shell, which hook load and prompt, and the project its
/// folder was in, if any.
#[derive(Debug, Clone, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ShellRecord {
    /// The shell that reported.
    shell: ShellId,
    /// The hook load and prompt the report was made at.
    stamp: ReportStamp,
    /// The project its folder was in, or `None` outside every project.
    #[builder(default)]
    project: Option<ProjectRoot>,
}
