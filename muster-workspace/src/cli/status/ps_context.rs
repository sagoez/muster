use std::{path::PathBuf, time::SystemTime};

use getset::{CopyGetters, Getters};
use muster_domain::{config::Stack, value::ProjectRoot};
use typed_builder::TypedBuilder;

use super::failure_reason::FailureReason;

/// What `ps` shows besides the service's statuses, and how.
#[derive(Debug, Getters, CopyGetters, TypedBuilder)]
pub struct PsContext<'a> {
    /// Whether each next step names its project, as one run with `-p`,
    /// under `--all` or outside the project needs.
    #[getset(get_copy = "pub")]
    name_project: bool,
    /// The stored stacks of the projects shown.
    #[getset(get = "pub")]
    stacks: &'a [Stack],
    /// Each project whose stack could not be read, with why.
    #[getset(get = "pub")]
    unread: &'a [(ProjectRoot, String)],
    /// Whether each process's stored command is shown under it.
    #[getset(get_copy = "pub")]
    commands: bool,
    /// Why each failed process failed.
    #[getset(get = "pub")]
    reasons: &'a [FailureReason],
    /// The home folder, shown as `~`.
    #[getset(get = "pub")]
    home: Option<PathBuf>,
    /// The time ages are measured to.
    #[getset(get_copy = "pub")]
    now: SystemTime,
}
