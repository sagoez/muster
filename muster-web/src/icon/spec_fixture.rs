use std::{path::PathBuf, time::Duration};

use muster_domain::{
    config::{Environment, ProcessSpec},
    process::{ProcessIcon, RestartPolicy, StopPolicy, StopSignal},
    value::{CommandLine, ProcessName, ProjectRoot, StopGrace},
};

/// The folder every fixture project lives in.
pub(super) const ROOT: &str = "/home/me/shop";
/// Seconds the fixture's processes get to stop.
const GRACE_SECONDS: u64 = 5;

/// The fixture project's root.
pub(super) fn root() -> ProjectRoot {
    ProjectRoot::try_new(PathBuf::from(ROOT)).unwrap()
}

/// A definition running `command` in `working_dir` under the root, wearing
/// the stored `icon` when given.
pub(super) fn spec(
    command: &str,
    working_dir: Option<&str>,
    icon: Option<ProcessIcon>,
) -> ProcessSpec {
    ProcessSpec::builder()
        .name(ProcessName::try_new("web").unwrap())
        .command(CommandLine::try_new(command).unwrap())
        .working_dir(working_dir.map(PathBuf::from))
        .icon(icon)
        .environment(Environment::empty())
        .autostart(false)
        .restart(RestartPolicy::Never)
        .stop(
            StopPolicy::builder()
                .signal(StopSignal::Terminate)
                .grace_period(StopGrace::try_new(Duration::from_secs(GRACE_SECONDS)).unwrap())
                .build(),
        )
        .build()
}
