use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{
    event_kind::EventKind, event_seq::EventSeq, launch_environment::LaunchEnvironment,
    process_action::ProcessAction, service_version::ServiceVersion, wait_spec::WaitSpec,
};
use crate::{
    adapter::workspace_request::WorkspaceRequest,
    domain::{
        occupancy::ShellRecord,
        value::{ProcessName, ProjectRoot},
    },
};

/// One request a client sends the service, as one JSON line. A `project` is
/// that project's folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServiceRequest {
    /// Introduces the client's build, so a client of another version can
    /// replace the service.
    Hello {
        /// The client's muster version.
        version: ServiceVersion,
    },
    /// Starts the project's `autostart` processes that are not running.
    Up {
        /// The project to bring up.
        project: ProjectRoot,
        /// The client's environment, which what it starts runs with.
        environment: LaunchEnvironment,
        /// A wait on each process it starts, answered as one batch once every
        /// wait ended.
        wait: Option<WaitSpec>,
    },
    /// Stops every process of the project.
    Down {
        /// The project to bring down.
        project: ProjectRoot,
    },
    /// Reports every process of one project, or of every loaded project.
    Ps {
        /// The project to report, or every loaded project when absent.
        project: Option<ProjectRoot>,
    },
    /// Applies a lifecycle action to one process.
    Control {
        /// The process's project.
        project: ProjectRoot,
        /// The process.
        name: ProcessName,
        /// What to do to it.
        action: ProcessAction,
        /// The client's environment, which a process it starts or restarts
        /// runs with.
        environment: LaunchEnvironment,
        /// A wait on the run the action leaves, registered in the same loop
        /// turn as the action so none of that run's output is missed; the
        /// answer is then how the wait ended.
        wait: Option<WaitSpec>,
    },
    /// Waits for a process's output, port, URL, state or readiness.
    Wait {
        /// The process's project.
        project: ProjectRoot,
        /// The process.
        name: ProcessName,
        /// What to wait for, and for how long.
        wait: WaitSpec,
    },
    /// Reads the project's lifecycle events after a cursor, waiting for one
    /// when there are none yet.
    Events {
        /// The project whose events to read.
        project: ProjectRoot,
        /// Events after this one count; without it, the newest few.
        since: Option<EventSeq>,
        /// The kinds to read, every kind when empty.
        kinds: Vec<EventKind>,
        /// Longest to wait for a new event when there is none.
        #[serde(with = "humantime_serde")]
        wait: Duration,
    },
    /// Finds where a running process's holder and log are.
    Locate {
        /// The process's project.
        project: ProjectRoot,
        /// The process.
        name: ProcessName,
    },
    /// Finds the log every run of a process writes to, whether or not it is
    /// running.
    Log {
        /// The process's project.
        project: ProjectRoot,
        /// The process.
        name: ProcessName,
    },
    /// Reads a process's visible screen.
    Screen {
        /// The process's project.
        project: ProjectRoot,
        /// The process.
        name: ProcessName,
    },
    /// Reports the recent notices of a project and of the service.
    Notices {
        /// The project whose notices to report.
        project: ProjectRoot,
    },
    /// A workspace request from an MCP client.
    Workspace {
        /// The project the client serves.
        project: ProjectRoot,
        /// The request.
        request: WorkspaceRequest,
    },
    /// The project's stored stack changed; a loaded project applies it.
    StackChanged {
        /// The project whose stack changed.
        project: ProjectRoot,
    },
    /// The stored settings changed; later launches use them.
    SettingsChanged,
    /// A shell's hook reports the project its folder is in at a prompt.
    ShellReport {
        /// The report: which shell, at which hook load and prompt, so an
        /// older report is dropped, and the project its folder is in.
        #[serde(with = "crate::adapter::wire::form")]
        record: ShellRecord,
        /// The shell's exported environment, which what entering a project
        /// starts runs with; sent only by a report that may enter one, and
        /// without it entering starts nothing.
        environment: Option<LaunchEnvironment>,
    },
    /// Exits so a newer service can take over; every process keeps running.
    Replace,
    /// Exits; every process keeps running.
    Stop,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{
        adapter::service::{PortBlock, PortHolder, PortOwner, Refusal, ServiceResponse, ShellNote},
        domain::{
            occupancy::{ShellId, testing::stamp},
            value::{LocalPort, Pid, ProjectName, StartToken},
        },
    };

    #[test]
    fn shell_report_and_notes_round_trip_as_json() {
        let report = ServiceRequest::ShellReport {
            record: ShellRecord::builder()
                .shell(
                    ShellId::builder()
                        .pid(Pid::try_new(4242).unwrap())
                        .start(StartToken::try_new(77).unwrap())
                        .build(),
                )
                .stamp(stamp(9, 3))
                .folder(PathBuf::from("/home/me/shop"))
                .project(Some(
                    ProjectRoot::try_new(PathBuf::from("/home/me/shop")).unwrap(),
                ))
                .build(),
            environment: Some(LaunchEnvironment::from_vars([(
                "PATH".into(),
                "/usr/bin".into(),
            )])),
        };
        let json = serde_json::to_string(&report).unwrap();
        assert_eq!(
            json,
            r#"{"shell-report":{"record":{"pid":4242,"start":77,"load":9,"load-pid":2,"seq":3,"folder":[47,104,111,109,101,47,109,101,47,115,104,111,112],"project":"/home/me/shop","held":[]},"environment":{"PATH":"/usr/bin"}}}"#
        );
        assert_eq!(
            serde_json::from_str::<ServiceRequest>(&json).unwrap(),
            report
        );

        let shop = ProjectName::try_new("shop").unwrap();
        let block = PortBlock::builder()
            .port(LocalPort::try_new(5173).unwrap())
            .holder(PortOwner::Supervised(
                PortHolder::builder()
                    .project(ProjectName::try_new("admin").unwrap())
                    .process(ProcessName::try_new("web").unwrap())
                    .build(),
            ))
            .build();
        let notes = ServiceResponse::ShellNotes(vec![
            ShellNote::Started {
                project: shop.clone(),
                names: vec![ProcessName::try_new("api").unwrap()],
            },
            ShellNote::Refused {
                project: shop,
                refusal: Refusal::builder()
                    .name(ProcessName::try_new("web").unwrap())
                    .reason(block.to_string())
                    .block(Some(block))
                    .build(),
            },
        ]);
        let json = serde_json::to_string(&notes).unwrap();
        assert!(json.starts_with(r#"{"shell-notes":[{"started":"#), "{json}");
        assert!(json.contains(r#"{"refused":"#), "{json}");
        assert_eq!(
            serde_json::from_str::<ServiceResponse>(&json).unwrap(),
            notes
        );
    }
}
