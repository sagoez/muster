//! `muster __shell-report`: what a shell's hook runs, detached, at every
//! prompt. It tells the service which project the shell is in, starting the
//! service only when that is a project, and leaves what the service answers
//! for the hook to print at the next prompt.

use std::path::Path;

use super::{cli_ports::CliPorts, shell_note_text::note_lines, shell_report_skip::ShellReportSkip};
use crate::{
    adapter::{
        client::{Asked, ClientError},
        service::{LaunchEnvironment, ServiceRequest, ServiceResponse},
    },
    domain::{
        occupancy::{ReportStamp, ShellRecord},
        value::Pid,
    },
};

/// Reports the shell running as `pid`, at the hook load and prompt `stamp`
/// gives, in the folder `cwd`, sending its environment only when it may
/// enter its project. A service of this version is told, one of another
/// version replaced and one being started waited for. With none, one is
/// started only when `cwd` is in a registered project; otherwise the record
/// is kept and the service asked again, so one that started meanwhile from
/// the old records still hears it. The answered notes are left for the
/// hook, then the record is written; a report that fails writes neither, so
/// the next prompt reports the same move again. A state directory not made
/// yet is made first when its parent is this user's, so a fresh install
/// keeps the record a later registration around the shell is told apart by.
/// It never prints.
///
/// # Errors
/// Returns the [`ShellReportSkip`] saying why nothing was written: the state
/// directory is another user's or cannot be made, the pid is no live shell,
/// a report as new is stored, or the database, service or shell files
/// failed.
pub fn shell_report(
    ports: &dyn CliPorts,
    pid: Pid,
    stamp: ReportStamp,
    cwd: &Path,
) -> Result<(), ShellReportSkip> {
    if !ports.claim_state_dir() {
        return Err(ShellReportSkip::NotOwnState);
    }
    let shells = ports.shell_registry()?;
    let shell = shells
        .identify(pid)
        .ok_or(ShellReportSkip::UnknownShell(pid))?;
    let previous = shells.record(&shell);
    if previous
        .as_ref()
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
    let in_project = project.is_some();
    let record = ShellRecord::builder()
        .shell(shell)
        .stamp(stamp)
        .folder(cwd.to_path_buf())
        .project(project)
        .build()
        .following(previous.as_ref());
    let request = ServiceRequest::ShellReport {
        environment: record
            .may_enter(previous.as_ref())
            .then(LaunchEnvironment::current),
        record: record.clone(),
    };
    let service = ports.service();
    let answer = match service.ask_if_running(&request)? {
        Asked::Answered(answer) => answer,
        Asked::Absent if !in_project => {
            shells.save_if_newer(&record)?;
            match service.ask_if_running(&request)? {
                Asked::Answered(answer) => answer,
                Asked::Absent => return Ok(()),
                Asked::OtherVersion | Asked::Starting => service.connect()?.send(&request)?,
            }
        },
        Asked::Absent | Asked::OtherVersion | Asked::Starting => {
            service.connect()?.send(&request)?
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
        ffi::OsStr,
        fs,
        os::unix::ffi::OsStrExt as _,
        path::PathBuf,
        process::{self, Command},
    };

    use rustix::process::geteuid;
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
    /// whether it runs, before and after the record is written, never
    /// started, and only the record is written.
    #[test]
    fn outside_any_project_with_no_service_only_the_record_is_written() {
        let (ports, _shop, scratch) = shop_ports(FakeService::default());

        shell_report(&ports, this_shell(), seq(1), scratch.path()).unwrap();

        let service = ports.fake_service();
        assert_eq!(service.asked().len(), 2);
        assert_eq!(service.connects(), 0);
        assert_eq!(
            record(&ports),
            Some(
                ShellRecord::builder()
                    .shell(identified(&ports))
                    .stamp(seq(1))
                    .folder(scratch.path().to_path_buf())
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

    /// CR-4: a leave reported while a service of another version runs
    /// replaces it, as an entering would, so the new service hears it.
    #[test]
    fn a_leave_replaces_a_service_of_another_version() {
        let served = ServedService::answering(ServiceResponse::ShellNotes(Vec::new()));
        let (ports, _shop, scratch) = shop_ports(
            FakeService::default()
                .asked_in_turn(vec![Asked::OtherVersion])
                .starting(&served),
        );

        shell_report(&ports, this_shell(), seq(1), scratch.path()).unwrap();

        assert_eq!(ports.fake_service().connects(), 1);
        let told = served.asked();
        assert!(
            matches!(&told[..], [ServiceRequest::ShellReport { record, .. }]
                if record.project().is_none()),
            "{told:?}"
        );
        assert_eq!(record(&ports).map(|record| *record.stamp()), Some(seq(1)));
    }

    /// Correctness M5: a leave reported while another client starts the
    /// service waits for it and tells it, whether the start was under way
    /// when asked or began between the record's write and the second ask.
    #[test]
    fn a_leave_while_a_service_starts_is_told_once_it_runs() {
        let no_notes = ServiceResponse::ShellNotes(Vec::new());
        for (asks, started) in [
            (vec![Asked::Starting], 1),
            (vec![Asked::Absent, Asked::Starting], 1),
            (vec![Asked::Absent, Asked::Answered(no_notes.clone())], 0),
        ] {
            let served = ServedService::answering(no_notes.clone());
            let (ports, _shop, scratch) = shop_ports(
                FakeService::default()
                    .asked_in_turn(asks.clone())
                    .starting(&served),
            );

            shell_report(&ports, this_shell(), seq(1), scratch.path()).unwrap();

            let service = ports.fake_service();
            assert_eq!(service.asked().len(), asks.len(), "{asks:?}");
            assert_eq!(service.connects(), started, "{asks:?}");
            assert_eq!(served.asked().len(), started, "{asks:?}");
            assert_eq!(
                record(&ports).map(|record| *record.stamp()),
                Some(seq(1)),
                "{asks:?}"
            );
        }
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
                    .folder(shop.clone())
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
                    .folder(shop.clone())
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

    /// CR-6: the exported environment goes only with a report whose project
    /// differs from the stored record, which may enter it; a prompt that
    /// stays, or leaves for no project, sends none.
    #[test]
    fn the_environment_is_sent_only_with_a_report_that_may_enter() {
        let (ports, shop, scratch) = shop_ports(FakeService::default().answering(started()));

        for (seq, folder) in [(1, &shop), (2, &shop), (3, &scratch.path().to_path_buf())] {
            shell_report(&ports, this_shell(), stamp(LOAD, seq), folder).unwrap();
        }

        let sent: Vec<bool> = ports
            .fake_service()
            .asked()
            .iter()
            .map(|request| {
                matches!(request, ServiceRequest::ShellReport {
                    environment: Some(_),
                    ..
                })
            })
            .collect();
        assert_eq!(sent, [true, false, false]);
    }

    /// Correctness M1: a project registered around a shell, at its root or
    /// nested in the project the shell was in, is told with no environment,
    /// so nothing starts; the nested one holds the outer project.
    #[test]
    fn a_project_registered_around_the_shell_is_told_as_no_entering() {
        let (ports, shop, _scratch) = shop_ports(FakeService::default().answering(started()));
        let admin = shop.join("admin");
        fs::create_dir_all(&admin).unwrap();
        let shop_root = ProjectRoot::try_new(shop).unwrap();
        let admin_root = ProjectRoot::try_new(admin.clone()).unwrap();

        shell_report(&ports, this_shell(), seq(1), &admin).unwrap();
        ports
            .store()
            .import(&admin_root, &[spec("panel", "true")], ImportMode::Strict)
            .unwrap();
        shell_report(&ports, this_shell(), seq(2), &admin).unwrap();

        let asked = ports.fake_service().asked();
        let [
            ServiceRequest::ShellReport {
                record: walked,
                environment: Some(_),
            },
            ServiceRequest::ShellReport {
                record: registered,
                environment: None,
            },
        ] = &asked[..]
        else {
            panic!("an entering, then a registration: {asked:?}");
        };
        assert_eq!(walked.project().as_ref(), Some(&shop_root));
        assert_eq!(registered.project().as_ref(), Some(&admin_root));
        assert_eq!(registered.held(), &[shop_root]);
        assert_eq!(record(&ports).as_ref(), Some(registered));
    }

    /// A report run as a user that does not own the state directory, as in
    /// a root shell that kept this user's home, opens and writes nothing
    /// there: no database read, no service asked, no shells folder made.
    #[test]
    fn a_report_as_another_user_touches_nothing() {
        let (ports, shop, _scratch) = shop_ports(FakeService::default().answering(started()));
        let ports = ports.running_as(geteuid().as_raw() + 1);
        let shells_dir = ports.shell_registry().unwrap().notes_dir();

        assert!(matches!(
            shell_report(&ports, this_shell(), seq(1), &shop),
            Err(ShellReportSkip::NotOwnState)
        ));

        assert!(ports.fake_service().asked().is_empty());
        assert_eq!(ports.fake_service().connects(), 0);
        assert!(!shells_dir.exists(), "no shells folder is made");
    }

    /// CR-6: on a fresh install the first report makes the state directory
    /// and keeps the record, so a project registered around the open shell
    /// afterwards is told with no environment and starts nothing.
    #[test]
    fn registration_around_an_open_shell_on_a_fresh_install_starts_nothing() {
        let (store, scratch) = fresh_store();
        let no_notes = ServiceResponse::ShellNotes(Vec::new());
        let ports = FakePorts::with_service(store, FakeService::default().answering(no_notes))
            .fresh_install();
        let folder = scratch.path().canonicalize().unwrap().join("shop");
        fs::create_dir_all(&folder).unwrap();

        shell_report(&ports, this_shell(), seq(1), &folder).unwrap();
        assert!(ports.state_dir().is_dir(), "the state directory is made");
        assert_eq!(record(&ports).map(|record| *record.stamp()), Some(seq(1)));

        ports
            .store()
            .import(
                &ProjectRoot::try_new(folder.clone()).unwrap(),
                &[spec("api", "true")],
                ImportMode::Strict,
            )
            .unwrap();
        shell_report(&ports, this_shell(), seq(2), &folder).unwrap();

        let asked = ports.fake_service().asked();
        assert!(
            matches!(&asked[..], [.., ServiceRequest::ShellReport {
                record,
                environment: None,
            }] if record.project().is_some()),
            "a registration, not an entering: {asked:?}"
        );
    }

    /// CR-6: with no state directory and a parent another user owns, the
    /// report makes and writes nothing.
    #[test]
    fn a_fresh_install_under_another_user_s_parent_makes_nothing() {
        let (ports, shop, _scratch) = shop_ports(FakeService::default().answering(started()));
        let ports = ports.fresh_install().running_as(geteuid().as_raw() + 1);

        assert!(matches!(
            shell_report(&ports, this_shell(), seq(1), &shop),
            Err(ShellReportSkip::NotOwnState)
        ));

        assert!(!ports.state_dir().exists());
        assert!(ports.fake_service().asked().is_empty());
    }

    /// S-M1: a shell that moves from a project into a folder whose name is
    /// not UTF-8 reports the leave, which travels as JSON, and its record
    /// keeps that folder whole. macOS file systems refuse such a name, so
    /// there is no such folder to leave into there.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn a_leave_into_a_folder_that_is_not_utf8_is_reported_and_kept() {
        let (ports, shop, scratch) = shop_ports(FakeService::default().answering(started()));
        let odd = scratch.path().join(OsStr::from_bytes(b"bad\xffdir"));
        fs::create_dir_all(&odd).unwrap();

        shell_report(&ports, this_shell(), seq(1), &shop).unwrap();
        shell_report(&ports, this_shell(), seq(2), &odd).unwrap();

        let asked = ports.fake_service().asked();
        let Some(leave @ ServiceRequest::ShellReport { record: told, .. }) = asked.last() else {
            panic!("a report: {asked:?}");
        };
        assert_eq!(told.project(), &None);
        let json = serde_json::to_string(leave).unwrap();
        assert_eq!(
            &serde_json::from_str::<ServiceRequest>(&json).unwrap(),
            leave
        );
        let stored = record(&ports).unwrap();
        assert_eq!(stored.folder(), &odd);
        assert_eq!(stored.project(), &None);
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
