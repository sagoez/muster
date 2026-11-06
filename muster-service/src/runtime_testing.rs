//! What the service runtime's tests and their fixtures share: bounds, the
//! test service's identity, and the processes and statuses they work with.

pub(super) use muster_domain::occupancy::testing::{test_root, test_shell};
use muster_domain::{
    config::{Environment, ProcessSpec},
    process::RestartPolicy,
    value::{CommandLine, ProcessName},
};
pub(super) use muster_os::test_wait::{POLL, WAIT, wait_until};
use muster_protocol::service::{LaunchEnvironment, ProcessStatus, ProjectStatus};

use super::supervisor::testing::test_stop;

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
