//! `muster` and `muster ps`: one marked line per process under a heading
//! naming the project, why each failed process failed, a count by state and
//! what to run next, drawn by the same renderer as every other report.

use std::{
    path::Path,
    time::{Duration, SystemTime},
};

use bytesize::ByteSize;
use muster_client::listening_ports;
use muster_domain::{
    config::{ProcessSpec, Stack},
    value::{ProjectName, printable},
};
use muster_protocol::service::{ExitReason, LastExit, ProcessStatus, ProjectStatus};
use strum::IntoEnumIterator;

use super::{
    block_hint::block_hint,
    columns::Columns,
    health::Health,
    next_step::{UP_IN_A_PROJECT, add_one, see_why, start_it, start_them},
    output::{blocked_text, occupancy_suffix, short_duration},
    ps_context::PsContext,
    report::{HINT_SEPARATOR, LABEL_GAP, SUMMARY_SEPARATOR, plain_line, plural},
    row::Row,
    row_kind::{RowKind, dimmed},
};

/// Opens a port among a process's facts.
const PORT_PREFIX: &str = ":";
/// Between the ports a process listens on.
const PORT_SEPARATOR: &str = " ";
/// What a starting process's line says until it is ready.
const NOT_READY_YET: &str = "not ready yet";
/// Opens how long a process has run.
const UP_WORD: &str = "up";
/// Follows the CPU share a process uses.
const PERCENT: &str = "%";
/// Digits after the point of that share.
const CPU_PRECISION: usize = 1;
/// Follows the count of a process's restarts: one.
const RESTART: &str = "restart";
/// Follows the count of a process's restarts: any other number.
const RESTARTS: &str = "restarts";
/// Says a client is attached; follows the count when there are several.
const ATTACHED: &str = "attached";
/// Opens the pid of a child that may still run unsupervised.
const PID_WORD: &str = "pid";
/// Follows how long ago a run ended.
const AGO: &str = "ago";
/// Stands for the home folder at the start of a project's folder.
const HOME_MARK: &str = "~";
/// Opens the dimmed line holding a process's command, with `--commands`.
const COMMAND_MARK: &str = "$ ";
/// `ps` of a project with no processes.
const NO_PROCESSES: &str = "no processes";
/// `ps --all` when the service has no project loaded.
const NO_PROJECTS: &str = "no projects are loaded";
/// Opens the line saying a project's stored processes could not be read;
/// why follows.
const UNREAD_STACK: &str =
    "could not read the stored processes, so commands and stored problems are not shown: ";

/// The `ps` lines of `statuses`, one block per project with a blank line
/// between them, styled for a terminal; anstream strips the styles where
/// they cannot show.
pub fn render_lines(statuses: &[ProjectStatus], context: &PsContext<'_>) -> String {
    if statuses.is_empty() {
        let empty = Row::unlabeled(RowKind::Idle, NO_PROJECTS);
        return format!(
            "{}\n{UP_IN_A_PROJECT}",
            plain_line(&empty, Columns::default())
        );
    }
    statuses
        .iter()
        .map(|status| project_block(status, context))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// One project's block: its heading, its processes, its problems, then the
/// count by state and what to run next.
fn project_block(status: &ProjectStatus, context: &PsContext<'_>) -> String {
    let named = context.name_project().then(|| status.project().name());
    let mut lines = vec![heading(status, context.home().as_deref()), String::new()];
    let processes = status.processes();
    if processes.is_empty() {
        let empty = Row::unlabeled(RowKind::Idle, NO_PROCESSES);
        lines.push(plain_line(&empty, Columns::default()));
        lines.extend(add_one(named));
        return lines.join("\n");
    }
    let root = status.project().root();
    let stack = context
        .stacks()
        .iter()
        .find(|stack| stack.project().root() == root);
    let healths: Vec<Health> = processes.iter().map(Health::of_process).collect();
    let mut rows: Vec<Row> = processes
        .iter()
        .zip(&healths)
        .map(|(process, health)| process_row(status, process, *health, stack, context))
        .collect();
    for (_, why) in context.unread().iter().filter(|(unread, _)| unread == root) {
        rows.push(Row::unlabeled(
            RowKind::Hint,
            format!("{UNREAD_STACK}{why}"),
        ));
    }
    rows.extend(
        stack
            .into_iter()
            .flat_map(|stack| stack.processes())
            .filter_map(ProcessSpec::stored_problem)
            .map(|problem| Row::unlabeled(RowKind::Hint, problem.to_string())),
    );
    let columns = Columns::of(&rows);
    lines.extend(rows.iter().map(|row| plain_line(row, columns)));
    lines.push(String::new());
    lines.push(summary(&healths));
    lines.extend(next_step(processes, &healths, stack, named).map(|step| printable(&step)));
    lines.join("\n")
}

/// `process`'s row: its mark, name, state word and facts, with why it
/// failed, what keeps it from starting and, with `--commands`, its stored
/// command under it.
fn process_row(
    status: &ProjectStatus,
    process: &ProcessStatus,
    health: Health,
    stack: Option<&Stack>,
    context: &PsContext<'_>,
) -> Row {
    let row = Row::stated(
        health.kind(),
        process.name().to_string(),
        health.to_string(),
        facts(process, health, context.now()).join(SUMMARY_SEPARATOR),
    );
    let reason = context
        .reasons()
        .iter()
        .find(|reason| reason.root() == status.project().root() && reason.name() == process.name())
        .map(|reason| reason.text().clone());
    let blocked = process.blocked_on().as_ref().map(|block| {
        let held = blocked_text(block);
        match block_hint(block, status.project().name()) {
            Some(hint) => format!("{held}{HINT_SEPARATOR}{hint}"),
            None => held,
        }
    });
    let command = context
        .commands()
        .then(|| {
            stack
                .and_then(|stack| stack.process(process.name()))
                .map(|spec| format!("{COMMAND_MARK}{}", spec.command().as_ref()))
        })
        .flatten();
    [reason, blocked, command]
        .into_iter()
        .flatten()
        .fold(row, Row::with_note)
}

/// What to run next in a project: the log of the first failed process,
/// unless it never launched and so logged nothing, its reason line saying
/// why; or, with every process down, `up` when one of them autostarts, else
/// a start of the first; naming the project when `project` is given.
fn next_step(
    processes: &[ProcessStatus],
    healths: &[Health],
    stack: Option<&Stack>,
    project: Option<&ProjectName>,
) -> Option<String> {
    let mut listed = processes.iter().zip(healths);
    if let Some((failed, _)) = listed
        .clone()
        .find(|(_, health)| **health == Health::Failed)
    {
        let launched = failed
            .last_exit()
            .as_ref()
            .is_none_or(|exit| exit.reason() != ExitReason::FailedToStart);
        return launched.then(|| see_why(failed.name(), project)).flatten();
    }
    if !listed.all(|(_, health)| health.is_down()) {
        return None;
    }
    let autostarts = processes.iter().any(|process| {
        stack
            .and_then(|stack| stack.process(process.name()))
            .is_some_and(|spec| *spec.autostart())
    });
    if autostarts {
        start_them(project)
    } else {
        start_it(processes.first()?.name(), project)
    }
}

/// The project's name, its folder with `~` for the home folder, and who is
/// in it.
fn heading(status: &ProjectStatus, home: Option<&Path>) -> String {
    let project = status.project();
    let root = project.root().as_ref();
    let shown = match home.and_then(|home| root.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => HOME_MARK.to_string(),
        Some(rest) => Path::new(HOME_MARK).join(rest).display().to_string(),
        None => root.display().to_string(),
    };
    let folder = format!("{shown}{}", occupancy_suffix(status));
    format!(
        "{}{LABEL_GAP}{}",
        printable(project.name().as_ref()),
        dimmed(&printable(&folder))
    )
}

/// The facts a process's line carries, only those with a value.
fn facts(process: &ProcessStatus, health: Health, now: SystemTime) -> Vec<String> {
    let mut facts = Vec::new();
    let ports = listening_ports(process);
    if !ports.is_empty() {
        facts.push(
            ports
                .iter()
                .map(|port| format!("{PORT_PREFIX}{port}"))
                .collect::<Vec<_>>()
                .join(PORT_SEPARATOR),
        );
    }
    if health == Health::Starting {
        facts.push(NOT_READY_YET.to_string());
    }
    if health == Health::Unsupervised
        && let Some(pid) = process.pid()
    {
        facts.push(format!("{PID_WORD} {pid}"));
    }
    if let Some(uptime) = process.uptime() {
        facts.push(format!("{UP_WORD} {}", short_duration(*uptime)));
    }
    if let Some(usage) = process.usage() {
        facts.push(format!("{:.CPU_PRECISION$}{PERCENT}", usage.cpu_percent()));
        facts.push(ByteSize::b(*usage.memory_bytes()).to_string());
    }
    let restarts = *process.restarts();
    if restarts > 0 {
        facts.push(format!(
            "{restarts} {}",
            plural(restarts as usize, RESTART, RESTARTS)
        ));
    }
    match *process.attached() {
        0 => {},
        1 => facts.push(ATTACHED.to_string()),
        many => facts.push(format!("{many} {ATTACHED}")),
    }
    if let Some(exit) = process.last_exit() {
        facts.push(exit.outcome().to_string());
        facts.extend(ended_ago(exit, now));
    }
    facts
}

/// How long before `now` the run `exit` ended, when that is known.
fn ended_ago(exit: &LastExit, now: SystemTime) -> Option<String> {
    let at = (*exit.at())?;
    let since = now.duration_since(at).unwrap_or(Duration::ZERO);
    Some(format!("{} {AGO}", short_duration(since)))
}

/// The count of `healths` by state, in [`Health`]'s order, zero counts left
/// out.
fn summary(healths: &[Health]) -> String {
    Health::iter()
        .filter_map(|health| {
            let count = healths.iter().filter(|&&each| each == health).count();
            (count > 0).then(|| format!("{count} {health}"))
        })
        .collect::<Vec<_>>()
        .join(SUMMARY_SEPARATOR)
}
