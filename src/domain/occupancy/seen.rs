use std::collections::BTreeSet;

use super::{ReportStamp, ShellRecord};
use crate::domain::value::ProjectRoot;

/// What the occupancy table knows of one shell: its newest report and the
/// projects that report counts it in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Seen {
    /// The newest report applied.
    pub(super) stamp: ReportStamp,
    /// The project it is in and those it holds.
    pub(super) projects: BTreeSet<ProjectRoot>,
}

impl Seen {
    /// What `record` says of its shell.
    pub(super) fn of(record: &ShellRecord) -> Self {
        Self {
            stamp: *record.stamp(),
            projects: record
                .project()
                .iter()
                .chain(record.held())
                .cloned()
                .collect(),
        }
    }
}
