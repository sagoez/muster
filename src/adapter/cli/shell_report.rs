//! `muster __shell-report`: what a shell's hook runs, detached, at every
//! prompt. It tells the service which project the shell is in, starting the
//! service only when that is a project, and leaves what the service answers
//! for the hook to print at the next prompt.

use std::path::Path;

use super::{cli_ports::CliPorts, shell_note_text::note_lines, shell_report_skip::ShellReportSkip};
use crate::{
    adapter::{
        client::ClientError,
        service::{LaunchEnvironment, ServiceRequest, ServiceResponse},
    },
    domain::{
        occupancy::{ReportStamp, ShellRecord},
        value::Pid,
    },
};

/// Reports the shell running as `pid`, at the hook load and prompt `stamp`
/// gives, in the folder `cwd`. A running service is always told; with none
/// running, one is started only when `cwd` is inside a registered project,
/// and otherwise only the shell's record is kept. The notes the service
/// answers are left for the hook, then the record is written; a report that
/// fails writes neither, so the next prompt reports the same move again. It
/// never prints.
///
/// # Errors
/// Returns the [`ShellReportSkip`] saying why nothing was written: the pid
/// is no live shell, a report as new is stored, or the database, service or
/// shell files failed.
pub fn shell_report(
    ports: &dyn CliPorts,
    pid: Pid,
    stamp: ReportStamp,
    cwd: &Path,
) -> Result<(), ShellReportSkip> {
    let shells = ports.shell_registry()?;
    let shell = shells
        .identify(pid)
        .ok_or(ShellReportSkip::UnknownShell(pid))?;
    if shells
        .record(&shell)
        .is_some_and(|stored| !stamp.supersedes(stored.stamp()))
    {
        return Err(ShellReportSkip::Stale);
    }
    let project = match ports.existing_state()? {
        Some(state) => state
            .project_containing(cwd)?
            .map(|project| project.root().clone()),
        None => None,
    };
    let entering = project.is_some();
    let record = ShellRecord::builder()
        .shell(shell)
        .stamp(stamp)
        .project(project)
        .build();
    let request = ServiceRequest::ShellReport {
        record: record.clone(),
        environment: LaunchEnvironment::current(),
    };
    let service = ports.service();
    let answer = match service.ask_if_running(&request)? {
        Some(answer) => answer,
        None if entering => service.connect()?.send(&request)?,
        None => {
            shells.save_if_newer(&record)?;
            return Ok(());
        },
    };
    let ServiceResponse::ShellNotes(notes) = answer else {
        return Err(ClientError::UnexpectedAnswer((&answer).into()).into());
    };
    shells.deliver(&shell, stamp, &note_lines(&notes))?;
    shells.save_if_newer(&record)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        process::{self, Command},
    };

    use tempfile::TempDir;

    use super::{super::fake_ports::FakePorts, *};
    use crate::{
        adapter::{
            client::{fake_service::FakeService, served_service::ServedService},
            service::ShellNote,
            store::test_support::{fresh_store, spec},
        },
        domain::{
            config::ImportMode,
            occupancy::{ShellId, testing::stamp},
            port::StackStore,
            value::{ProcessName, ProjectName, ProjectRoot},
        },
    };

    /// Suffix of a note file.
    const NOTE_SUFFIX: &str = ".note";
    /// The hook load the tests report from.
    const LOAD: u64 = 3;

    /// The pid of this test, a live process to report as a shell.
    fn this_shell() -> Pid {
        Pid::try_new(process::id()).unwrap()
    }

    /// The stamp of prompt `n` of [`LOAD`].
    fn seq(n: u64) -> ReportStamp {
        stamp(LOAD, n)
    }

    /// Ports over a fresh database holding the project `shop` with one
    /// process, reaching `service`; the project's folder, and the scratch
    /// directory holding both.
    fn shop_ports(service: FakeService) -> (FakePorts, PathBuf, TempDir) {
        let (store, scratch) = fresh_store();
        let folder = scratch.path().canonicalize().unwrap().join("shop");
        fs::create_dir_all(&folder).unwrap();
        store
            .import(
                &ProjectRoot::try_new(folder.clone()).unwrap(),
                &[spec("api", "true")],
                ImportMode::Strict,
            )
            .unwrap();
        (FakePorts::with_service(store, service), folder, scratch)
    }

    /// The shell this test runs as, as the ports' shell files identify it.
    fn identified(ports: &FakePorts) -> ShellId {
        ports
            .shell_registry()
            .unwrap()
            .identify(this_shell())
            .unwrap()
    }

    /// The record kept for this test's shell.
    fn record(ports: &FakePorts) -> Option<ShellRecord> {
        let shells = ports.shell_registry().unwrap();
        shells.record(&shells.identify(this_shell()).unwrap())
    }

    /// The text of every note left for the hook, in name order.
    fn notes(ports: &FakePorts) -> Vec<String> {
        let dir = ports.shell_registry().unwrap().notes_dir();
        let Ok(entries) = fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut paths: Vec<PathBuf> = entries
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.to_string_lossy().ends_with(NOTE_SUFFIX))
            .collect();
        paths.sort();
        paths
            .iter()
            .map(|path| fs::read_to_string(path).unwrap())
            .collect()
    }

    /// The note the service answers when entering `shop` started `api`.
    fn started() -> ServiceResponse {
        ServiceResponse::ShellNotes(vec![ShellNote::Started {
            project: ProjectName::try_new("shop").unwrap(),
            names: vec![ProcessName::try_new("api").unwrap()],
        }])
    }

    /// Outside every project, with no service running, the service is asked
    /// whether it runs, never started, and only the record is written.
    #[test]
    fn outside_any_project_with_no_service_only_the_record_is_written() {
        let (ports, _shop, scratch) = shop_ports(FakeService::default());

        shell_report(&ports, this_shell(), seq(1), scratch.path()).unwrap();

        let service = ports.fake_service();
        assert_eq!(service.asked().len(), 1);
        assert_eq!(service.connects(), 0);
        assert_eq!(
            record(&ports),
            Some(
                ShellRecord::builder()
                    .shell(identified(&ports))
                    .stamp(seq(1))
                    .project(None)
                    .build()
            )
        );
        assert!(notes(&ports).is_empty());
    }

    /// Inside a project, with no service running, the service is started
    /// and told, its notes are left for the hook, and the record names the
    /// project.
    #[test]
    fn inside_a_project_with_no_service_the_service_is_started_and_notes_delivered() {
        let served = ServedService::answering(started());
        let (ports, shop, _scratch) = shop_ports(FakeService::default().starting(&served));
        let inside = shop.join("src");
        fs::create_dir_all(&inside).unwrap();

        shell_report(&ports, this_shell(), seq(4), &inside).unwrap();

        let root = ProjectRoot::try_new(shop).unwrap();
        assert_eq!(ports.fake_service().connects(), 1);
        let told = served.asked();
        assert!(
            matches!(&told[..], [ServiceRequest::ShellReport { record, .. }]
                if *record.shell() == identified(&ports)
                    && *record.stamp() == seq(4)
                    && record.project().as_ref() == Some(&root)),
            "{told:?}"
        );
        assert_eq!(notes(&ports), vec!["muster: shop: starting api\n"]);
        assert_eq!(
            record(&ports).and_then(|record| record.project().clone()),
            Some(root)
        );
    }

    /// A running service is told without a connection, and its notes and
    /// the record are written.
    #[test]
    fn a_running_service_is_told_without_starting_one() {
        let (ports, shop, _scratch) = shop_ports(FakeService::default().answering(started()));

        shell_report(&ports, this_shell(), seq(2), &shop).unwrap();

        assert_eq!(ports.fake_service().asked().len(), 1);
        assert_eq!(ports.fake_service().connects(), 0);
        assert_eq!(notes(&ports), vec!["muster: shop: starting api\n"]);
        assert_eq!(record(&ports).map(|record| *record.stamp()), Some(seq(2)));
    }

    /// A service that cannot be started, or that refuses the report,
    /// leaves no record and no note, so the next prompt reports again.
    #[test]
    fn a_failed_delivery_writes_neither_record_nor_notes() {
        let (unreachable, shop, _scratch) = shop_ports(FakeService::default());

        assert!(matches!(
            shell_report(&unreachable, this_shell(), seq(1), &shop),
            Err(ShellReportSkip::Service(_))
        ));
        assert_eq!(unreachable.fake_service().connects(), 1);
        assert_eq!(record(&unreachable), None);
        assert!(notes(&unreachable).is_empty());

        let (refusing, shop, _scratch) =
            shop_ports(FakeService::default().answering(ServiceResponse::Ok));

        assert!(matches!(
            shell_report(&refusing, this_shell(), seq(1), &shop),
            Err(ShellReportSkip::Service(ClientError::UnexpectedAnswer(_)))
        ));
        assert_eq!(record(&refusing), None);
        assert!(notes(&refusing).is_empty());
    }

    /// A report no newer than the stored one is dropped before the service
    /// is asked or started.
    #[test]
    fn a_stale_seq_never_contacts_the_service() {
        let (ports, shop, _scratch) = shop_ports(FakeService::default());
        ports
            .shell_registry()
            .unwrap()
            .save_if_newer(
                &ShellRecord::builder()
                    .shell(identified(&ports))
                    .stamp(seq(5))
                    .build(),
            )
            .unwrap();

        for stale in [seq(5), seq(3)] {
            assert!(matches!(
                shell_report(&ports, this_shell(), stale, &shop),
                Err(ShellReportSkip::Stale)
            ));
        }

        assert!(ports.fake_service().asked().is_empty());
        assert_eq!(ports.fake_service().connects(), 0);
        assert_eq!(record(&ports).map(|record| *record.stamp()), Some(seq(5)));
    }

    /// A shell that `exec`s itself loads the hook again and counts from
    /// one: that report is newer than every report of the load before, so
    /// it is told and recorded.
    #[test]
    fn a_report_of_a_new_hook_load_is_told_whatever_its_seq() {
        let (ports, shop, _scratch) = shop_ports(FakeService::default().answering(started()));
        ports
            .shell_registry()
            .unwrap()
            .save_if_newer(
                &ShellRecord::builder()
                    .shell(identified(&ports))
                    .stamp(stamp(LOAD, 500))
                    .build(),
            )
            .unwrap();

        shell_report(&ports, this_shell(), stamp(LOAD + 1, 1), &shop).unwrap();

        assert_eq!(ports.fake_service().asked().len(), 1);
        assert_eq!(
            record(&ports).map(|record| *record.stamp()),
            Some(stamp(LOAD + 1, 1))
        );
        assert_eq!(notes(&ports), vec!["muster: shop: starting api\n"]);
    }

    /// A pid no process runs as is no shell: nothing is asked or written.
    #[test]
    fn an_unknown_pid_does_nothing() {
        let (ports, shop, _scratch) = shop_ports(FakeService::default());
        let mut child = Command::new("true").spawn().unwrap();
        let gone = Pid::try_new(child.id()).unwrap();
        child.wait().unwrap();

        assert!(matches!(
            shell_report(&ports, gone, seq(1), &shop),
            Err(ShellReportSkip::UnknownShell(pid)) if pid == gone
        ));

        assert!(ports.fake_service().asked().is_empty());
        assert_eq!(ports.fake_service().connects(), 0);
        let shells = ports.shell_registry().unwrap();
        assert!(shells.records().is_empty());
        assert!(notes(&ports).is_empty());
    }
}
