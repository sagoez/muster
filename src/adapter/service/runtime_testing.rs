//! What the service runtime's tests and their fixtures share: bounds, the
//! test service's identity, and the processes and statuses they work with.

use super::{
    launch_environment::LaunchEnvironment, process_status::ProcessStatus,
    project_status::ProjectStatus, supervisor::testing::test_stop,
};
pub(super) use crate::adapter::test_wait::{POLL, WAIT, wait_until};
use crate::domain::{
    config::{Environment, ProcessSpec},
    occupancy::ShellId,
    process::RestartPolicy,
    value::{CommandLine, Pid, ProcessName, StartToken},
};

/// Prefix of every test scratch directory.
pub(super) const SCRATCH_PREFIX: &str = "ms-";
/// The version the test services report.
pub(super) const VERSION: &str = "0.0.0-test";
/// The binary the test services report running.
pub(super) const TEST_BINARY: &str = "/usr/bin/muster-test";
/// A child that runs until it is killed.
pub(super) const SLEEPER: &str = "sleep 600";

/// A process `name` running [`SLEEPER`], starting with the project when
/// `autostart`.
pub(super) fn sleeper(name: &str, autostart: bool) -> ProcessSpec {
    ProcessSpec::builder()
        .name(ProcessName::try_new(name).unwrap())
        .command(CommandLine::try_new(SLEEPER).unwrap())
        .environment(Environment::empty())
        .autostart(autostart)
        .restart(RestartPolicy::Never)
        .stop(test_stop())
        .build()
}

/// Processes `(name, autostart)`, each running [`SLEEPER`].
pub(super) fn commands(entries: &[(&str, bool)]) -> Vec<ProcessSpec> {
    entries
        .iter()
        .map(|(name, autostart)| sleeper(name, *autostart))
        .collect()
}

/// The shell running as `pid`, started at `start`.
pub(super) fn test_shell(pid: u32, start: u64) -> ShellId {
    ShellId::builder()
        .pid(Pid::try_new(pid).unwrap())
        .start(StartToken::try_new(start).unwrap())
        .build()
}

/// A client asking with no environment of its own.
pub(super) fn no_env() -> LaunchEnvironment {
    LaunchEnvironment::default()
}

/// The status of the process `name` in `status`, if it has a row.
pub(super) fn find<'a>(status: &'a ProjectStatus, name: &str) -> Option<&'a ProcessStatus> {
    status
        .processes()
        .iter()
        .find(|process| process.name().as_ref() == name)
}

/// The status of the process `name` in `status`.
pub(super) fn process<'a>(status: &'a ProjectStatus, name: &str) -> &'a ProcessStatus {
    find(status, name).unwrap()
}
