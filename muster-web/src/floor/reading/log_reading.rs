use getset::Getters;
use muster_domain::value::{ProcessName, ProjectName};
use typed_builder::TypedBuilder;

use super::log_stat::LogStat;

/// What statting one watched process's log found.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct LogReading {
    /// Its project.
    project: ProjectName,
    /// The process.
    name: ProcessName,
    /// The stat, none when it has no log yet.
    stat: Option<LogStat>,
}
