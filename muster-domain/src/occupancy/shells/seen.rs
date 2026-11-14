use std::collections::BTreeSet;

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::{
    occupancy::{ReportStamp, ShellRecord},
    value::ProjectRoot,
};

/// What the occupancy table knows of one shell: its newest report and the
/// projects that report counts it in.
#[derive(Debug, Clone, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct Seen {
    /// The newest report applied.
    stamp: ReportStamp,
    /// The project it is in and those it holds.
    projects: BTreeSet<ProjectRoot>,
}

impl Seen {
    /// What `record` says of its shell.
    pub(crate) fn of(record: &ShellRecord) -> Self {
        Self::builder()
            .stamp(*record.stamp())
            .projects(
                record
                    .project()
                    .iter()
                    .chain(record.held())
                    .cloned()
                    .collect(),
            )
            .build()
    }
}
