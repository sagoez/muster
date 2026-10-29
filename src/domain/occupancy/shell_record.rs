use getset::Getters;
use typed_builder::TypedBuilder;

use super::{ReportSeq, ShellId};
use crate::domain::value::ProjectRoot;

/// A shell's last report as it is kept between services: which shell, which
/// prompt, and the project its folder was in, if any.
#[derive(Debug, Clone, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ShellRecord {
    /// The shell that reported.
    shell: ShellId,
    /// The prompt the report was made at.
    seq: ReportSeq,
    /// The project its folder was in, or `None` outside every project.
    #[builder(default)]
    project: Option<ProjectRoot>,
}
