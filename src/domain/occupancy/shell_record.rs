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

impl ShellRecord {
    /// Whether this report may enter its project and start it: it names a
    /// project `previous`, the shell's last stored report, did not.
    pub fn may_enter(&self, previous: Option<&Self>) -> bool {
        self.project.is_some() && previous.is_none_or(|previous| previous.project != self.project)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::domain::{
        occupancy::testing::stamp,
        value::{Pid, StartToken},
    };

    /// The record of prompt `seq` in the project `/home/me/<project>`.
    fn at(seq: u64, project: Option<&str>) -> ShellRecord {
        ShellRecord::builder()
            .shell(
                ShellId::builder()
                    .pid(Pid::try_new(100).unwrap())
                    .start(StartToken::try_new(1).unwrap())
                    .build(),
            )
            .stamp(stamp(1, seq))
            .project(
                project.map(|name| {
                    ProjectRoot::try_new(PathBuf::from("/home/me").join(name)).unwrap()
                }),
            )
            .build()
    }

    /// Only a report naming a project the last one did not may enter it;
    /// a prompt in the same project, or one outside every project, may not.
    #[test]
    fn only_a_report_naming_a_new_project_may_enter() {
        assert!(at(1, Some("shop")).may_enter(None));
        assert!(at(2, Some("shop")).may_enter(Some(&at(1, None))));
        assert!(at(2, Some("blog")).may_enter(Some(&at(1, Some("shop")))));
        assert!(!at(2, Some("shop")).may_enter(Some(&at(1, Some("shop")))));
        assert!(!at(2, None).may_enter(Some(&at(1, Some("shop")))));
        assert!(!at(1, None).may_enter(None));
    }
}
