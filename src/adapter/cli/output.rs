//! How process commands print: the `ps` table, the report of an action, the
//! notices list, and the `--json` forms of each.

use std::{
    cmp::Ordering,
    io,
    path::Path,
    time::{Duration, SystemTime},
};

use bytesize::ByteSize;
use semver::Version;
use serde::Serialize;
use serde_json::{Value, json};
use strum::EnumMessage;
use unicode_width::UnicodeWidthStr;

use super::{
    action::Action,
    action_report::ActionReport,
    attach_end::AttachEnd,
    error::CliError,
    notice_view::NoticeView,
    outcome::Outcome,
    process_outcome::ProcessOutcome,
    report::{Report, Row, RowKind, tally},
    running_service::RunningService,
    service_status::ServiceStatus,
};
use crate::{
    adapter::service::{Notice, NoticeLevel, ProcessStatus, ProjectStatus},
    domain::{
        process::ProcessState,
        pty::{ExitCause, ExitOutcome, SignalName},
        value::ProcessName,
    },
};

/// Column headers of the `ps` table, in order.
const PS_HEADERS: [&str; 10] = [
    "NAME",
    "KIND",
    "STATE",
    "PID",
    "UPTIME",
    "RESTARTS",
    "CPU",
    "MEM",
    "ATTACHED",
    "LAST EXIT",
];
/// Gap between two table columns.
const COLUMN_GAP: &str = "  ";
/// Shown in a table cell with nothing to show.
const EMPTY_CELL: &str = "-";
/// Indent of a project's table under its heading in `ps --all`.
const PROJECT_INDENT: &str = "  ";
/// Largest units an uptime or age is shown in: `1h 2m`, not `1h 2m 3s`.
const DURATION_UNITS: usize = 2;
/// Separates the units of a formatted duration.
const DURATION_UNIT_SEPARATOR: char = ' ';
/// An exit observed without a status or signal, in the `ps` table.
const UNKNOWN_EXIT: &str = "unknown";
/// Opens the line after a detach, before the process's name.
const DETACHED_FROM: &str = "detached from";
/// The command that attaches to a process again, before its quoted name.
const ATTACH_COMMAND: &str = "muster attach";
/// Follows that command in the line after a detach.
const ATTACH_AGAIN: &str = "attaches again";
/// Follows the process's name in the line after it exited while attached.
const EXITED: &str = "exited";
/// Marks an exit that failed.
const FAILED: &str = "failed";
/// `ps --all` when the service has no project loaded.
const NO_PROJECTS: &str =
    "no projects are loaded; `muster up` in a project folder starts its processes";
/// `ps` of a project with no processes.
const NO_PROCESSES: &str = "no processes; add some to muster.yml";
/// `muster notices` with nothing to report.
const NO_NOTICES: &str = "no notices";
/// Label of a notice about the whole service.
const SERVICE_LABEL: &str = "service";
/// Label of a notice about the whole project.
const PROJECT_LABEL: &str = "project";
/// The `up` report when the service started and refused nothing.
const NOTHING_TO_START: &str = "nothing to start: every process set to autostart is running";
/// How to start what `up` leaves alone.
const UP_HINT: &str = "`muster start <name>` starts a process that does not autostart";
/// The `down` report when the service stopped nothing.
const NOTHING_RUNNING: &str = "nothing was running";
/// How to stop the agents `down` leaves running.
const DOWN_AGENTS_HINT: &str = "agents keep running; `muster down --agents` stops them too";
/// Heading of the notices an action raised about processes it did not act on.
const OTHER_NOTICES: &str = "other notices, about processes this command did not touch:";
/// What follows an outcome the command stopped waiting for.
const PS_FOLLOW_UP: &str = "when the wait ended; `muster ps` shows how it ends";
/// What follows a crash its restart policy is about to retry.
const RESTARTING_FOLLOW_UP: &str = "restarting under its restart policy";
/// Title of the `notices` report, after the prefix.
const NOTICES_TITLE: &str = "notices";
/// Title of the `service` reports, after the prefix.
const SERVICE_TITLE: &str = "service";
/// Between the parts of one outcome's detail.
const DETAIL_SEPARATOR: &str = ": ";
/// Between a notice's age and its message.
const AGE_SEPARATOR: &str = " ago: ";
/// Prefix of a report's title.
const TITLE_PREFIX: &str = "muster";
/// The command word of `muster attach`.
const ATTACH_COMMAND_WORD: &str = "attach";
/// The command word of `muster logs`.
const LOGS_COMMAND_WORD: &str = "logs";
/// Between a command and its project in a title.
const TITLE_SEPARATOR: &str = " · ";
/// Between the words of an action's kebab-case name.
const KEBAB_SEPARATOR: char = '-';
/// Between the words of a command as typed.
const WORD_SEPARATOR: &str = " ";
/// Start of the line saying the service runs, before its version.
const SERVICE_RUNNING: &str = "running, version";
/// What happens to an older service.
const SERVICE_REPLACED_HINT: &str =
    "the next muster command replaces the service, and every process keeps running";
/// What happens with a newer service.
const SERVICE_NEWER_HINT: &str =
    "older than the service, so its commands are refused; run the newer muster";
/// `service status` when no service runs.
const SERVICE_NOT_RUNNING: &str = "not running; the next muster command starts it";
/// `service stop` after stopping a running service.
const SERVICE_STOPPED: &str = "stopped the muster service";
/// `service stop` when no service ran.
const SERVICE_WAS_NOT_RUNNING: &str = "the muster service was not running";
/// What stopping the service leaves alone.
const PROCESSES_KEEP_RUNNING: &str =
    "every process keeps running; the next muster command starts the service again";
/// Label of the service socket row.
const SOCKET_LABEL: &str = "socket";
/// Label of the service log row.
const LOG_LABEL: &str = "log";
/// Label of the service binary row.
const BINARY_LABEL: &str = "binary";
/// Printed if a failure could not be turned into JSON, which keeps `--json`
/// output parseable even then.
const FALLBACK_FAILURE_JSON: &str =
    r#"{"error":{"kind":"io","message":"the failure could not be described","candidates":[]}}"#;

/// The human `ps` table of `statuses`. With `all`, each project's table sits
/// under a heading naming the project; without it, the one project's table
/// stands alone.
pub fn render_status(statuses: &[ProjectStatus], all: bool) -> String {
    if statuses.is_empty() {
        return NO_PROJECTS.to_string();
    }
    if !all {
        return statuses
            .iter()
            .map(|status| project_table(status, ""))
            .collect::<Vec<_>>()
            .join("\n\n");
    }
    statuses
        .iter()
        .map(|status| {
            format!(
                "{}\n{}",
                status.project().as_ref().display(),
                project_table(status, PROJECT_INDENT)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The table of one project's processes, each line indented by `indent`.
fn project_table(status: &ProjectStatus, indent: &str) -> String {
    if status.processes().is_empty() {
        return format!("{indent}{NO_PROCESSES}");
    }
    let mut rows = vec![PS_HEADERS.map(str::to_string).to_vec()];
    rows.extend(status.processes().iter().map(process_cells));
    table(&rows, indent)
}

/// The cells of one process's `ps` row.
fn process_cells(process: &ProcessStatus) -> Vec<String> {
    let usage = process.usage().as_ref();
    vec![
        process.name().to_string(),
        process.kind().to_string(),
        process.state().to_string(),
        cell(process.pid().map(|pid| pid.to_string())),
        cell(process.uptime().map(short_duration)),
        process.restarts().to_string(),
        cell(usage.map(|usage| format!("{:.1}%", usage.cpu_percent()))),
        cell(usage.map(|usage| ByteSize::b(*usage.memory_bytes()).to_string())),
        cell((*process.attached() > 0).then(|| process.attached().to_string())),
        cell(process.last_exit().as_ref().map(exit_cell)),
    ]
}

/// How a process last ended, in a `ps` cell: `status 1`, `signal SIGKILL`.
fn exit_cell(exit: &ExitOutcome) -> String {
    exit.summary().unwrap_or_else(|| UNKNOWN_EXIT.to_string())
}

/// `value`, or the empty-cell marker.
fn cell(value: Option<String>) -> String {
    value.unwrap_or_else(|| EMPTY_CELL.to_string())
}

/// `duration` to the second, in its largest units.
fn short_duration(duration: Duration) -> String {
    let whole = Duration::from_secs(duration.as_secs());
    humantime::format_duration(whole)
        .to_string()
        .split(DURATION_UNIT_SEPARATOR)
        .take(DURATION_UNITS)
        .collect::<Vec<_>>()
        .join(&DURATION_UNIT_SEPARATOR.to_string())
}

/// `rows` aligned in columns by display width, so names with wide or
/// combining characters line up, each line indented by `indent`.
fn table(rows: &[Vec<String>], indent: &str) -> String {
    let columns = rows.first().map_or(0, Vec::len);
    let widths: Vec<usize> = (0..columns)
        .map(|column| {
            rows.iter()
                .map(|row| UnicodeWidthStr::width(row[column].as_str()))
                .max()
                .unwrap_or(0)
        })
        .collect();
    rows.iter()
        .map(|row| {
            let line = row
                .iter()
                .zip(&widths)
                .map(|(value, width)| {
                    let pad = width - UnicodeWidthStr::width(value.as_str());
                    format!("{value}{}", " ".repeat(pad))
                })
                .collect::<Vec<_>>()
                .join(COLUMN_GAP);
            format!("{indent}{}", line.trim_end())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The human report of `report`: one row per process saying what became of
/// it, with what supervision said about it, then any other notices as
/// context.
pub fn action_rows(report: &ActionReport) -> Report {
    let mut rows = vec![Row::unlabeled(
        RowKind::Dim,
        report.project().display().to_string(),
    )];
    let acted: Vec<Row> = report
        .processes()
        .iter()
        .filter(|process| process.outcome() != Outcome::KeptRunning)
        .map(|process| {
            Row::labeled(
                outcome_kind(process.outcome()),
                process.name().to_string(),
                outcome_detail(report, process),
            )
        })
        .collect();
    let summary = tally(&acted);
    if acted.is_empty() {
        rows.extend(nothing_done(report.action()));
    }
    rows.extend(acted);
    if report
        .processes()
        .iter()
        .any(|process| process.outcome() == Outcome::KeptRunning)
    {
        rows.push(Row::unlabeled(RowKind::Hint, DOWN_AGENTS_HINT));
    }
    if !report.other_notices().is_empty() {
        rows.push(Row::unlabeled(RowKind::Dim, OTHER_NOTICES));
        rows.extend(report.other_notices().iter().map(|notice| {
            Row::labeled(
                RowKind::Hint,
                notice_label(notice),
                notice.message().clone(),
            )
        }));
    }
    let titled = Report::new(action_title(report.action(), report.project()), rows)
        .with_failure(report.failed());
    match summary {
        Some(summary) => titled.with_summary(summary),
        None => titled,
    }
}

/// The title a command acting on the project at `project` opens with: the
/// command and the project's folder, such as `muster up · shop`.
pub fn action_title(action: Action, project: &Path) -> String {
    project_title(&action_words(action), project)
}

/// The title `muster attach` opens with, such as `muster attach · shop`.
pub fn attach_title(project: &Path) -> String {
    project_title(ATTACH_COMMAND_WORD, project)
}

/// The title `muster logs` opens its question with, such as
/// `muster logs · shop`.
pub fn logs_title(project: &Path) -> String {
    project_title(LOGS_COMMAND_WORD, project)
}

/// The title the `muster service` commands open with.
pub fn service_title() -> String {
    format!("{TITLE_PREFIX} {SERVICE_TITLE}")
}

/// `muster <command> · <folder>`, for the project whose config is `project`.
fn project_title(command: &str, project: &Path) -> String {
    let folder = project.parent().and_then(Path::file_name).map_or_else(
        || project.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    format!("{TITLE_PREFIX} {command}{TITLE_SEPARATOR}{folder}")
}

/// How the command line spells `action`: `agent new`, not `agent-new`.
fn action_words(action: Action) -> String {
    action.to_string().replace(KEBAB_SEPARATOR, WORD_SEPARATOR)
}

/// What to say when an action touched no process.
fn nothing_done(action: Action) -> Vec<Row> {
    match action {
        Action::Up => vec![
            Row::unlabeled(RowKind::Ok, NOTHING_TO_START),
            Row::unlabeled(RowKind::Dim, UP_HINT),
        ],
        _ => vec![Row::unlabeled(RowKind::Ok, NOTHING_RUNNING)],
    }
}

/// The glyph kind of `outcome`.
fn outcome_kind(outcome: Outcome) -> RowKind {
    if outcome.is_failure() {
        RowKind::Fail
    } else if outcome.is_pending() {
        RowKind::Hint
    } else {
        RowKind::Ok
    }
}

/// What became of `process` in `report`, in words: the outcome's message,
/// how its child last ended or why it was left alone, then each thing
/// supervision said about it that the words so far do not already say, and
/// the pid of any child that runs.
fn outcome_detail(report: &ActionReport, process: &ProcessOutcome) -> String {
    let outcome = process.outcome();
    let status = process.status().as_ref();
    let mut parts = vec![outcome.get_message().unwrap_or_default().to_string()];
    match outcome {
        Outcome::Unchanged => {
            if let Some(status) = status {
                parts[0] = format!("{} {}", parts[0], status.state());
            }
        },
        Outcome::Crashed | Outcome::Exited => {
            if let Some(exit) = status.and_then(|status| status.last_exit().as_ref()) {
                parts.push(exit_words(exit.status(), exit.signal()));
            }
            if status.is_some_and(|status| status.state() == ProcessState::Restarting) {
                parts.push(RESTARTING_FOLLOW_UP.to_string());
            }
        },
        Outcome::StillStarting | Outcome::StillStopping => {
            parts[0] = format!("{} {PS_FOLLOW_UP}", parts[0]);
        },
        Outcome::Refused => {
            if let Some(reason) = process.reason() {
                parts[0].clone_from(reason);
            }
        },
        _ => {},
    }
    for notice in report
        .notices()
        .iter()
        .filter(|notice| notice.process().as_ref() == Some(process.name()))
    {
        let said = parts.join(DETAIL_SEPARATOR);
        if !notice
            .message()
            .split(DETAIL_SEPARATOR)
            .all(|part| said.contains(part))
        {
            parts.push(notice.message().clone());
        }
    }
    let detail = parts.join(DETAIL_SEPARATOR);
    match status.and_then(|status| status.pid()) {
        Some(pid) => format!("{detail} (pid {pid})"),
        None => detail,
    }
}

/// How a process ended, from its exit status or signal, in the words the
/// service's notices use.
fn exit_words(status: &Option<u32>, signal: &Option<SignalName>) -> String {
    let outcome = match (status, signal) {
        (Some(0), _) => ExitOutcome::Succeeded,
        (Some(status), _) => ExitOutcome::Failed(ExitCause::Status(*status)),
        (None, Some(signal)) => ExitOutcome::Failed(ExitCause::Signal(signal.clone())),
        (None, None) => ExitOutcome::FAILED_UNKNOWN,
    };
    outcome.to_string()
}

/// The glyph kind of a notice of `level`.
fn level_kind(level: NoticeLevel) -> RowKind {
    match level {
        NoticeLevel::Error => RowKind::Fail,
        NoticeLevel::Warning | NoticeLevel::Attention => RowKind::Hint,
    }
}

/// What a notice concerns: its process, its project, or the service.
fn notice_label(notice: &NoticeView) -> String {
    match (notice.process(), notice.project()) {
        (Some(process), _) => process.to_string(),
        (None, Some(_)) => PROJECT_LABEL.to_string(),
        (None, None) => SERVICE_LABEL.to_string(),
    }
}

/// The human list of `notices` of `project`, oldest first, each with how
/// long before `now` it was raised.
pub fn notices_rows(project: &Path, notices: &[Notice], now: SystemTime) -> Report {
    let mut rows = vec![Row::unlabeled(RowKind::Dim, project.display().to_string())];
    if notices.is_empty() {
        rows.push(Row::unlabeled(RowKind::Ok, NO_NOTICES));
    }
    rows.extend(notices.iter().map(|notice| {
        let age = now
            .duration_since(*notice.at())
            .map(short_duration)
            .unwrap_or_else(|_| EMPTY_CELL.to_string());
        let view = NoticeView::from(notice);
        Row::labeled(
            level_kind(*notice.level()),
            notice_label(&view),
            format!("{age}{AGE_SEPARATOR}{}", notice.message()),
        )
    }));
    Report::new(format!("{TITLE_PREFIX} {NOTICES_TITLE}"), rows)
}

/// The human `muster service status` report.
pub fn service_rows(status: &ServiceStatus) -> Report {
    let mut rows = Vec::new();
    match status.version() {
        Some(version) => {
            rows.push(Row::unlabeled(
                RowKind::Ok,
                format!("{SERVICE_RUNNING} {version}"),
            ));
            let running = RunningService::builder()
                .version(Some(version.clone()))
                .binary(status.binary().clone())
                .build();
            let hint = Version::parse(status.client_version())
                .ok()
                .map(|ours| running.compared_with(&ours))
                .and_then(|ordering| match ordering {
                    Ordering::Greater => Some(SERVICE_REPLACED_HINT),
                    Ordering::Less => Some(SERVICE_NEWER_HINT),
                    Ordering::Equal => None,
                });
            if let Some(hint) = hint {
                rows.push(Row::unlabeled(
                    RowKind::Hint,
                    format!("this muster is version {}; {hint}", status.client_version()),
                ));
            }
        },
        None => rows.push(Row::unlabeled(RowKind::Hint, SERVICE_NOT_RUNNING)),
    }
    if let Some(binary) = status.binary() {
        rows.push(Row::labeled(
            RowKind::Ok,
            BINARY_LABEL,
            binary.display().to_string(),
        ));
    }
    rows.push(Row::labeled(
        RowKind::Ok,
        SOCKET_LABEL,
        status.socket().display().to_string(),
    ));
    rows.push(Row::labeled(
        RowKind::Ok,
        LOG_LABEL,
        status.log().display().to_string(),
    ));
    rows.extend(
        status
            .projects()
            .iter()
            .map(|project| Row::labeled(RowKind::Ok, PROJECT_LABEL, project.display().to_string())),
    );
    Report::new(service_title(), rows)
}

/// The human `muster service stop` report, after a service that was
/// `running` was asked to stop.
pub fn service_stop_rows(running: bool) -> Report {
    let detail = if running {
        SERVICE_STOPPED
    } else {
        SERVICE_WAS_NOT_RUNNING
    };
    Report::new(service_title(), vec![
        Row::unlabeled(RowKind::Ok, detail),
        Row::unlabeled(RowKind::Dim, PROCESSES_KEEP_RUNNING),
    ])
}

/// The line `muster attach` prints once the terminal is given back: that it
/// detached and how to attach again, or how the process `name` exited. A
/// signal that ended the client leaves nothing to say.
pub fn attach_end_line(name: &ProcessName, end: &AttachEnd) -> Option<String> {
    match end {
        AttachEnd::Detached => Some(format!(
            "{DETACHED_FROM} {name}; `{ATTACH_COMMAND} {}` {ATTACH_AGAIN}",
            name.shell_quoted()
        )),
        AttachEnd::Exited(ExitOutcome::Succeeded) => Some(format!("{name} {EXITED}")),
        AttachEnd::Exited(outcome) => Some(match outcome.summary() {
            Some(summary) => format!("{name} {EXITED} ({FAILED}, {summary})"),
            None => format!("{name} {EXITED} ({FAILED})"),
        }),
        AttachEnd::Signalled(_) => None,
    }
}

/// `value` as the stable, pretty-printed JSON every `--json` command prints.
///
/// # Errors
/// Returns a [`CliError`] when `value` cannot be serialized.
pub fn json<T: Serialize>(value: &T) -> Result<String, CliError> {
    serde_json::to_string_pretty(value).map_err(|error| CliError::Output(io::Error::other(error)))
}

/// `error` as the JSON object a `--json` command prints when it fails: its
/// kind, its message and the process names the person could have meant.
pub fn failure_json(error: &CliError) -> String {
    serde_json::to_string_pretty(&failure(error, &error.to_string()))
        .unwrap_or_else(|_| FALLBACK_FAILURE_JSON.to_string())
}

/// `error` as the JSON object a failure is reported as: its kind, `message`
/// and the process names the caller could have meant.
pub fn failure(error: &CliError, message: &str) -> Value {
    json!({
        "error": {
            "kind": error.kind(),
            "message": message,
            "candidates": error.candidates(),
        }
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{
        adapter::{
            cli::{
                ending_signal::EndingSignal, failure_kind::FailureKind, process_view::ProcessView,
                project_view::ProjectView, report::plain,
            },
            service::{ProcessUsage, ProjectKey, ServiceErrorKind},
        },
        domain::{
            process::{ActivityState, ProcessKind},
            value::ProcessName,
        },
    };

    /// The project every fixture belongs to.
    const PROJECT: &str = "/home/me/shop/muster.yml";
    /// A second project for `ps --all`.
    const OTHER_PROJECT: &str = "/home/me/blog/muster.yml";
    /// A fixed instant fixtures are timed from.
    const EPOCH_OFFSET: Duration = Duration::from_secs(1_800_000_000);
    /// A stop grace fixtures run with.
    const GRACE: Duration = Duration::from_secs(5);

    /// The process name `name`.
    fn name(name: &str) -> ProcessName {
        ProcessName::try_new(name).unwrap()
    }

    /// Process `named` of `kind` in `state`, with usage while it has `pid`,
    /// and `last_exit` while it does not.
    fn process(
        named: &str,
        kind: ProcessKind,
        state: ProcessState,
        pid: Option<u32>,
        last_exit: Option<ExitOutcome>,
    ) -> ProcessStatus {
        ProcessStatus::builder()
            .name(name(named))
            .kind(kind)
            .state(state)
            .activity(ActivityState::Idle)
            .pid(pid)
            .uptime(pid.map(|_| Duration::from_millis(3_723_400)))
            .restarts(0)
            .usage(pid.map(|pid| {
                ProcessUsage::builder()
                    .pid(pid)
                    .cpu_percent(2.25)
                    .memory_bytes(128 * 1024 * 1024)
                    .build()
            }))
            .attached(0)
            .last_exit(last_exit)
            .stop_grace((kind == ProcessKind::Command).then_some(GRACE))
            .build()
    }

    /// A project with an agent, an attached command that restarted, a
    /// crashed terminal, a starting command and one killed by a signal.
    fn shop() -> ProjectStatus {
        let dev = process(
            "dev server",
            ProcessKind::Command,
            ProcessState::Running,
            Some(4123),
            None,
        );
        let dev = ProcessStatus::builder()
            .name(dev.name().clone())
            .kind(*dev.kind())
            .state(*dev.state())
            .activity(ActivityState::Working)
            .pid(*dev.pid())
            .uptime(*dev.uptime())
            .restarts(2)
            .usage(*dev.usage())
            .attached(1)
            .last_exit(None)
            .stop_grace(Some(GRACE))
            .build();
        ProjectStatus::builder()
            .project(ProjectKey::new(PathBuf::from(PROJECT)))
            .processes(vec![
                process(
                    "claude",
                    ProcessKind::Agent,
                    ProcessState::Running,
                    Some(4100),
                    None,
                ),
                dev,
                process(
                    "café",
                    ProcessKind::Terminal,
                    ProcessState::Crashed,
                    None,
                    Some(ExitOutcome::Failed(ExitCause::Status(127))),
                ),
                process(
                    "worker",
                    ProcessKind::Command,
                    ProcessState::Starting,
                    None,
                    None,
                ),
                process(
                    "cron",
                    ProcessKind::Command,
                    ProcessState::Crashed,
                    None,
                    Some(ExitOutcome::Failed(ExitCause::Signal(
                        SignalName::try_new("sigkill").unwrap(),
                    ))),
                ),
            ])
            .build()
    }

    /// A project with no processes.
    fn blog() -> ProjectStatus {
        ProjectStatus::builder()
            .project(ProjectKey::new(PathBuf::from(OTHER_PROJECT)))
            .processes(vec![])
            .build()
    }

    /// A notice of the shop project about `process`.
    fn notice(process: Option<&str>, level: NoticeLevel, message: &str) -> Notice {
        Notice::builder()
            .at(SystemTime::UNIX_EPOCH + EPOCH_OFFSET)
            .project(Some(ProjectKey::new(PathBuf::from(PROJECT))))
            .process(process.map(name))
            .level(level)
            .message(message.to_string())
            .build()
    }

    #[test]
    fn ps_table_snapshot() {
        insta::assert_snapshot!("ps_table", render_status(&[shop()], false));
        insta::assert_snapshot!("ps_table_all", render_status(&[shop(), blog()], true));
        insta::assert_snapshot!("ps_table_none_loaded", render_status(&[], true));
    }

    #[test]
    fn ps_json_is_stable() {
        let views: Vec<ProjectView> = [shop()].iter().map(ProjectView::from).collect();
        insta::assert_snapshot!("ps_json", json(&views).unwrap());
    }

    /// Columns line up by display width, so a name with an accent or a wide
    /// character does not push its row out of line.
    #[test]
    fn ps_columns_align_by_display_width() {
        let rendered = render_status(&[shop()], false);
        let kind_column: Vec<usize> = rendered
            .lines()
            .map(|line| {
                let byte = line
                    .find("agent")
                    .or_else(|| line.find("command"))
                    .or_else(|| line.find("terminal"))
                    .or_else(|| line.find("KIND"))
                    .unwrap();
                UnicodeWidthStr::width(&line[..byte])
            })
            .collect();
        assert!(
            kind_column.windows(2).all(|pair| pair[0] == pair[1]),
            "{rendered}"
        );
    }

    /// What became of process `named`, listed as `status`.
    fn outcome(named: &str, outcome: Outcome, status: Option<ProcessStatus>) -> ProcessOutcome {
        ProcessOutcome::builder()
            .name(name(named))
            .outcome(outcome)
            .status(status.as_ref().map(ProcessView::from))
            .build()
    }

    /// An `up` that started one process, saw another crash, and was refused
    /// a third, while a process it did not touch raised an error.
    fn up_report() -> ActionReport {
        ActionReport::builder()
            .project(PathBuf::from(PROJECT))
            .action(Action::Up)
            .processes(vec![
                outcome(
                    "dev server",
                    Outcome::Started,
                    Some(process(
                        "dev server",
                        ProcessKind::Command,
                        ProcessState::Running,
                        Some(4123),
                        None,
                    )),
                ),
                outcome(
                    "café",
                    Outcome::Crashed,
                    Some(process(
                        "café",
                        ProcessKind::Terminal,
                        ProcessState::Crashed,
                        None,
                        Some(ExitOutcome::Failed(ExitCause::Status(127))),
                    )),
                ),
                ProcessOutcome::builder()
                    .name(name("claude"))
                    .outcome(Outcome::Refused)
                    .reason(Some(
                        "left stopped: its agent session cannot be resumed".to_string(),
                    ))
                    .status(None)
                    .build(),
            ])
            .notices(vec![NoticeView::from(&notice(
                Some("café"),
                NoticeLevel::Error,
                "crashed: exited with status 127",
            ))])
            .other_notices(vec![NoticeView::from(&notice(
                Some("cron"),
                NoticeLevel::Error,
                "crashed: ended by signal: Killed",
            ))])
            .build()
    }

    #[test]
    fn up_report_snapshot() {
        let report = up_report();
        assert!(report.failed());
        let rows = action_rows(&report);
        assert_eq!(rows.title(), "muster up · shop");
        assert_eq!(rows.summary().as_deref(), Some("1 ok · 2 failures"));
        assert!(rows.failed(), "the frame closes as a failure");
        insta::assert_snapshot!("up_report_plain", plain(&rows));
        insta::assert_snapshot!("up_report_json", json(&report).unwrap());
    }

    /// Every framed command's plain fallback, which a pipe, `NO_COLOR` or a
    /// dumb terminal gets: one line per fact under the project's path, with
    /// no title, summary or frame.
    #[test]
    fn every_framed_command_falls_back_to_plain_lines() {
        let running = |named: &str, kind: ProcessKind| {
            Some(process(
                named,
                kind,
                ProcessState::Running,
                Some(4200),
                None,
            ))
        };
        let framed = [
            (
                Action::Up,
                Outcome::Started,
                running("api", ProcessKind::Command),
            ),
            (Action::Down, Outcome::Stopped, None),
            (
                Action::Start,
                Outcome::Started,
                running("api", ProcessKind::Command),
            ),
            (Action::Stop, Outcome::Stopped, None),
            (
                Action::Restart,
                Outcome::Restarted,
                running("api", ProcessKind::Command),
            ),
            (
                Action::Run,
                Outcome::Started,
                running("api", ProcessKind::Command),
            ),
            (
                Action::AgentNew,
                Outcome::Started,
                running("claude", ProcessKind::Agent),
            ),
            (Action::AgentClose, Outcome::Stopped, None),
            (Action::AgentReopen, Outcome::Queued, None),
        ];
        let reports: Vec<Report> = framed
            .into_iter()
            .map(|(action, settled, status)| {
                let named = status
                    .as_ref()
                    .map_or_else(|| "api".to_string(), |status| status.name().to_string());
                let report = ActionReport::builder()
                    .project(PathBuf::from(PROJECT))
                    .action(action)
                    .processes(vec![outcome(&named, settled, status)])
                    .notices(vec![])
                    .other_notices(vec![])
                    .build();
                let rows = action_rows(&report);
                assert!(!rows.failed(), "{action}");
                rows
            })
            .chain([service_stop_rows(true)])
            .collect();
        let titles: Vec<&str> = reports
            .iter()
            .map(|report| report.title().as_str())
            .collect();
        assert_eq!(titles, vec![
            "muster up · shop",
            "muster down · shop",
            "muster start · shop",
            "muster stop · shop",
            "muster restart · shop",
            "muster run · shop",
            "muster agent new · shop",
            "muster agent close · shop",
            "muster agent reopen · shop",
            "muster service",
        ]);
        let printed: Vec<String> = reports.iter().map(plain).collect();
        insta::assert_snapshot!("framed_plain", printed.join("\n\n"));
    }

    /// The notice that says why a process was left alone belongs to the
    /// report and is printed once, on that process's row, not again as
    /// context.
    #[test]
    fn a_refusal_notice_is_printed_once_on_its_row() {
        let reason = "cannot start: the pid 4242 may still run unsupervised";
        let report = ActionReport::builder()
            .project(PathBuf::from(PROJECT))
            .action(Action::Up)
            .processes(vec![
                ProcessOutcome::builder()
                    .name(name("api"))
                    .outcome(Outcome::Refused)
                    .reason(Some(reason.to_string()))
                    .status(None)
                    .build(),
            ])
            .notices(vec![])
            .other_notices(vec![NoticeView::from(&notice(
                Some("api"),
                NoticeLevel::Error,
                reason,
            ))])
            .build()
            .claiming_notices_about(&[name("api")]);

        assert!(report.other_notices().is_empty());
        let rendered = plain(&action_rows(&report));
        assert_eq!(rendered.matches(reason).count(), 1, "{rendered}");
        assert!(!rendered.contains("other notices"), "{rendered}");
    }

    /// A notice about a process the command did not act on is shown as
    /// context and never fails the command.
    #[test]
    fn a_neighbours_crash_does_not_fail_the_command() {
        let report = ActionReport::builder()
            .project(PathBuf::from(PROJECT))
            .action(Action::Stop)
            .processes(vec![outcome("api", Outcome::Stopped, None)])
            .notices(vec![])
            .other_notices(vec![NoticeView::from(&notice(
                Some("cron"),
                NoticeLevel::Error,
                "crashed: exited with status 1",
            ))])
            .build();

        assert!(!report.failed());
        let rendered = plain(&action_rows(&report));
        assert!(rendered.contains("other notices"), "{rendered}");
    }

    #[test]
    fn an_up_that_started_nothing_says_so_and_how_to_start_the_rest() {
        let report = ActionReport::builder()
            .project(PathBuf::from(PROJECT))
            .action(Action::Up)
            .processes(vec![])
            .notices(vec![])
            .other_notices(vec![])
            .build();
        assert!(!report.failed());
        insta::assert_snapshot!("up_nothing", plain(&action_rows(&report)));
    }

    #[test]
    fn a_down_that_kept_agents_says_how_to_stop_them() {
        let report = ActionReport::builder()
            .project(PathBuf::from(PROJECT))
            .action(Action::Down)
            .processes(vec![
                outcome("dev server", Outcome::Stopped, None),
                outcome(
                    "claude",
                    Outcome::KeptRunning,
                    Some(process(
                        "claude",
                        ProcessKind::Agent,
                        ProcessState::Running,
                        Some(4100),
                        None,
                    )),
                ),
            ])
            .notices(vec![])
            .other_notices(vec![])
            .build();
        assert!(!report.failed());
        insta::assert_snapshot!("down_kept_agents", plain(&action_rows(&report)));
    }

    /// A stop still waiting on its process when the wait ends fails and says
    /// how to follow it up.
    #[test]
    fn a_stop_still_stopping_fails_and_says_so() {
        let report = ActionReport::builder()
            .project(PathBuf::from(PROJECT))
            .action(Action::Stop)
            .processes(vec![outcome(
                "dev server",
                Outcome::StillStopping,
                Some(process(
                    "dev server",
                    ProcessKind::Command,
                    ProcessState::Stopping,
                    Some(4123),
                    None,
                )),
            )])
            .notices(vec![])
            .other_notices(vec![])
            .build();
        assert!(report.failed());
        insta::assert_snapshot!("stop_still_stopping", plain(&action_rows(&report)));
    }

    #[test]
    fn notices_snapshot() {
        let now = SystemTime::UNIX_EPOCH + EPOCH_OFFSET + Duration::from_secs(75);
        let notices = vec![
            notice(
                Some("api"),
                NoticeLevel::Error,
                "crashed: exited with status 1",
            ),
            notice(None, NoticeLevel::Warning, "the config was reloaded"),
        ];
        insta::assert_snapshot!(
            "notices",
            plain(&notices_rows(Path::new(PROJECT), &notices, now))
        );
        insta::assert_snapshot!(
            "notices_none",
            plain(&notices_rows(Path::new(PROJECT), &[], now))
        );
        let views: Vec<NoticeView> = notices.iter().map(NoticeView::from).collect();
        insta::assert_snapshot!("notices_json", json(&views).unwrap());
    }

    /// An unknown name prints the valid names, quoted so the spaces in one
    /// show, and `--json` carries them as candidates.
    #[test]
    fn unknown_name_prints_valid_names() {
        let error = CliError::UnknownProcess {
            name: "dev".to_string(),
            project: PathBuf::from(PROJECT),
            valid: vec![name("dev server"), name("café"), name("it's")],
        };

        insta::assert_snapshot!("unknown_name", error.to_string());
        insta::assert_snapshot!("unknown_name_json", failure_json(&error));
    }

    #[test]
    fn a_refusal_names_what_to_try() {
        let busy = CliError::Refused {
            kind: ServiceErrorKind::Busy,
            message: "the service is shutting down".to_string(),
        };
        assert!(busy.to_string().contains("run the command again"));
        assert_eq!(busy.kind(), FailureKind::Busy);
    }

    /// A name with an apostrophe still yields a hint that pastes into a
    /// shell as that name.
    #[test]
    fn a_hint_quotes_a_name_for_the_shell() {
        let error = CliError::NotRunning(name("it's"));
        let message = error.to_string();
        let command = message.split('`').nth(1).expect("the hint names a command");

        assert_eq!(shlex::split(command).unwrap(), vec![
            "muster", "start", "it's"
        ]);
    }

    /// `service status` says whether this muster would replace the service
    /// or is refused by it.
    #[test]
    fn the_service_status_names_an_older_or_newer_client() {
        let status = |version: &str| {
            ServiceStatus::builder()
                .running(true)
                .version(Some(version.to_string()))
                .binary(Some(PathBuf::from("/usr/bin/muster")))
                .client_version("0.9.1".to_string())
                .socket(PathBuf::from("/s/service.sock"))
                .log(PathBuf::from("/s/service.log"))
                .projects(vec![])
                .build()
        };

        assert!(plain(&service_rows(&status("0.9.0"))).contains(SERVICE_REPLACED_HINT));
        assert!(plain(&service_rows(&status("0.10.0"))).contains(SERVICE_NEWER_HINT));
        assert!(!plain(&service_rows(&status("0.9.1"))).contains("this muster"));
    }

    /// The line after an attach says it detached or how the process exited,
    /// and nothing after a signal ended the client.
    #[test]
    fn an_attach_ends_with_what_became_of_it() {
        let api = name("api");
        let line = |end: AttachEnd| attach_end_line(&api, &end);

        assert_eq!(
            line(AttachEnd::Detached).as_deref(),
            Some("detached from api; `muster attach 'api'` attaches again")
        );
        let spaced = name("dev server");
        let again = attach_end_line(&spaced, &AttachEnd::Detached).unwrap();
        let command = again.split('`').nth(1).unwrap();
        assert_eq!(shlex::split(command).unwrap(), vec![
            "muster",
            "attach",
            "dev server"
        ]);
        assert_eq!(
            line(AttachEnd::Exited(ExitOutcome::Succeeded)).as_deref(),
            Some("api exited")
        );
        assert_eq!(
            line(AttachEnd::Exited(ExitOutcome::Failed(ExitCause::Status(2)))).as_deref(),
            Some("api exited (failed, status 2)")
        );
        assert_eq!(
            line(AttachEnd::Exited(ExitOutcome::FAILED_UNKNOWN)).as_deref(),
            Some("api exited (failed)")
        );
        assert_eq!(line(AttachEnd::Signalled(EndingSignal::Hangup)), None);
    }
}
