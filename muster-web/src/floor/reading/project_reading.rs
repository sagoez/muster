use getset::Getters;
use muster_domain::{config::Stack, process::ProcessIcon, project::Project};
use typed_builder::TypedBuilder;

use super::stacked_process::StackedProcess;
use crate::{ManifestReader, project_icon, resolve};

/// One registered project as the floor reads it: its stored stack, with the
/// icons resolved for it and for each process.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub(crate) struct ProjectReading {
    /// The project.
    project: Project,
    /// The icon the project wears.
    icon: ProcessIcon,
    /// Its processes, in stack order; none when its stack did not read.
    processes: Vec<StackedProcess>,
    /// Whether its stack did not read, so its processes are not known.
    unreadable: bool,
}

impl ProjectReading {
    /// The reading of `project`, whose stack did not read: no processes, and
    /// the icon of a plain terminal.
    pub(crate) fn unread(project: &Project) -> Self {
        Self::builder()
            .project(project.clone())
            .icon(ProcessIcon::Terminal)
            .processes(Vec::new())
            .unreadable(true)
            .build()
    }

    /// The reading of `stack`, its icons resolved over `manifests`.
    pub(crate) fn of(stack: &Stack, manifests: &dyn ManifestReader) -> Self {
        let root = stack.project().root();
        let processes: Vec<StackedProcess> = stack
            .processes()
            .iter()
            .map(|spec| {
                StackedProcess::builder()
                    .name(spec.name().clone())
                    .icon(resolve(spec, root, manifests))
                    .checked(spec.ready().is_some())
                    .build()
            })
            .collect();
        let icons: Vec<ProcessIcon> = processes.iter().map(|process| *process.icon()).collect();
        Self::builder()
            .project(stack.project().clone())
            .icon(project_icon(&icons, root, manifests))
            .processes(processes)
            .unreadable(false)
            .build()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use muster_domain::{
        process::{ReadyCheck, ReadyProbe},
        value::{LocalPort, PositiveDuration, ProjectName},
    };

    use super::*;
    use crate::icon::{
        fake_manifests::FakeManifests,
        spec_fixture::{root, spec},
    };

    /// The port a fixture readiness check probes.
    const CHECKED_PORT: u16 = 3000;
    /// Seconds a fixture readiness check may take.
    const CHECK_SECONDS: u64 = 30;

    /// A stack reads with each process's icon and the project's, and with
    /// whether its runs wait on a readiness check.
    #[test]
    fn a_stack_reads_with_its_icons_and_checks() {
        let project = Project::builder()
            .root(root())
            .name(ProjectName::try_new("shop").unwrap())
            .build();
        let check = ReadyCheck::builder()
            .probe(ReadyProbe::Port(LocalPort::try_new(CHECKED_PORT).unwrap()))
            .timeout(PositiveDuration::try_new(Duration::from_secs(CHECK_SECONDS)).unwrap())
            .build();
        let stack = Stack::builder()
            .project(project.clone())
            .processes(vec![spec("cargo run", None, None)])
            .build();
        let checked = Stack::builder()
            .project(project.clone())
            .processes(vec![spec("cargo run", None, None).with_ready(Some(check))])
            .build();

        let reading = ProjectReading::of(&stack, &FakeManifests::default());
        let checked = ProjectReading::of(&checked, &FakeManifests::default());

        assert_eq!(reading.project(), &project);
        assert_eq!(*reading.icon(), ProcessIcon::Rust);
        assert_eq!(reading.processes().len(), 1);
        assert_eq!(reading.processes()[0].name().as_ref(), "web");
        assert_eq!(*reading.processes()[0].icon(), ProcessIcon::Rust);
        assert!(!*reading.processes()[0].checked());
        assert!(*checked.processes()[0].checked());
    }
}
