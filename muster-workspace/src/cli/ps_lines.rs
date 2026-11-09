//! `muster` and `muster ps`: one marked line per process under a heading
//! naming the project, why each failed process failed, a count by state and
//! what to run next.

use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use bytesize::ByteSize;
use getset::Getters;
use muster_client::listening_ports;
use muster_domain::{
    config::{ProcessSpec, Stack},
    pty::{ExitCause, ExitOutcome},
    value::{ProcessName, ProjectRoot, printable},
};
use muster_protocol::service::{LastExit, ProcessStatus, ProjectStatus};
use strum::IntoEnumIterator;
use typed_builder::TypedBuilder;
use unicode_width::UnicodeWidthStr;

use super::{
    block_hint::block_hint,
    health::Health,
    next_step::{ADD_ONE, START_THEM, UP_IN_A_PROJECT, see_why},
    output::{HINT_SEPARATOR, blocked_text, occupancy_suffix, short_duration},
    report::{LABEL_GAP, NOTE_INDENT, SUMMARY_SEPARATOR, plural},
    row_kind::{RowKind, dimmed},
};

/// Between the facts on a process's line.
const FACT_SEPARATOR: &str = " · ";
/// Opens a port among a process's facts.
const PORT_PREFIX: &str = ":";
/// Between the ports a process listens on.
const PORT_SEPARATOR: &str = " ";
/// What a starting process's line says until it is ready.
const NOT_READY_YET: &str = "not ready yet";
/// Opens how long a process has run.
const UP_WORD: &str = "up";
/// Follows the count of a process's restarts.
const RESTART: &str = "restart";
const RESTARTS: &str = "restarts";
/// Says a client is attached; follows the count when there are several.
const ATTACHED: &str = "attached";
/// Opens an exit status.
const EXIT_WORD: &str = "exit";
/// What an exit observed without a status or signal reads as.
const UNKNOWN_EXIT: &str = "no exit status";
/// Follows how long ago a run ended.
const AGO: &str = "ago";
/// The status a successful exit has.
const SUCCESS_STATUS: u32 = 0;
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

/// What `ps` shows besides the service's statuses, and how.
#[derive(Debug, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct PsContext<'a> {
    /// Whether every loaded project is shown, so each next step names its
    /// project.
    all: bool,
    /// The stored stacks of the projects shown.
    stacks: &'a [Stack],
    /// Each project whose stack could not be read, with why.
    unread: &'a [(ProjectRoot, String)],
    /// Whether each process's stored command is shown under it.
    commands: bool,
    /// Why each failed process failed, by project and name, each cut to fit
    /// its line.
    reasons: &'a [(ProjectRoot, ProcessName, String)],
    /// The home folder, shown as `~`.
    home: Option<PathBuf>,
    /// The time ages are measured to.
    now: SystemTime,
}

/// The `ps` lines of `statuses`, one block per project with a blank line
/// between them, styled for a terminal; anstream strips the styles where
/// they cannot show.
pub fn render_lines(statuses: &[ProjectStatus], context: &PsContext<'_>) -> String {
    if statuses.is_empty() {
        return format!("{}\n{UP_IN_A_PROJECT}", marked(RowKind::Idle, NO_PROJECTS));
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
    let mut lines = vec![heading(status, context.home().as_deref()), String::new()];
    let processes = status.processes();
    if processes.is_empty() {
        lines.push(marked(RowKind::Idle, NO_PROCESSES));
        lines.push(ADD_ONE.to_string());
        return lines.join("\n");
    }
    let root = status.project().root();
    let stack = context
        .stacks()
        .iter()
        .find(|stack| stack.project().root() == root);
    let healths: Vec<Health> = processes.iter().map(Health::of_process).collect();
    let name_width = widest(
        processes
            .iter()
            .map(|process| printable(process.name().as_ref())),
    );
    let word_width = widest(healths.iter().map(ToString::to_string));
    for (process, health) in processes.iter().zip(&healths) {
        let name = printable(process.name().as_ref());
        let word = health.to_string();
        let facts = facts(process, *health, *context.now()).join(FACT_SEPARATOR);
        let line = format!(
            "{name}{}{LABEL_GAP}{word}{}{LABEL_GAP}{facts}",
            pad(&name, name_width),
            pad(&word, word_width),
        );
        lines.push(marked(health.kind(), line.trim_end()));
        let reason = context
            .reasons()
            .iter()
            .find(|(at, named, _)| at == root && named == process.name())
            .map(|(_, _, reason)| reason.clone());
        let blocked = process.blocked_on().as_ref().map(|block| {
            let held = blocked_text(block);
            match block_hint(block, status.project().name()) {
                Some(hint) => format!("{held}{HINT_SEPARATOR}{hint}"),
                None => held,
            }
        });
        let command = context.commands().then(|| {
            stack
                .and_then(|stack| stack.process(process.name()))
                .map(|spec| format!("{COMMAND_MARK}{}", spec.command().as_ref()))
        });
        for note in [reason, blocked, command.flatten()].into_iter().flatten() {
            lines.push(format!("{NOTE_INDENT}{}", dimmed(&printable(&note))));
        }
    }
    for (_, why) in context.unread().iter().filter(|(unread, _)| unread == root) {
        lines.push(marked(
            RowKind::Hint,
            &printable(&format!("{UNREAD_STACK}{why}")),
        ));
    }
    for problem in stack
        .into_iter()
        .flat_map(|stack| stack.processes())
        .filter_map(ProcessSpec::stored_problem)
    {
        lines.push(marked(RowKind::Hint, &printable(&problem.to_string())));
    }
    lines.push(String::new());
    lines.push(summary(&healths));
    let failed = processes
        .iter()
        .zip(&healths)
        .find(|(_, health)| **health == Health::Failed);
    let next = match failed {
        Some((process, _)) => see_why(
            process.name(),
            context.all().then(|| status.project().name()),
        ),
        None if healths.iter().all(|health| health.is_down()) => Some(START_THEM.to_string()),
        None => None,
    };
    lines.extend(next.map(|step| printable(&step)));
    lines.join("\n")
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
    if let Some(uptime) = process.uptime() {
        facts.push(format!("{UP_WORD} {}", short_duration(*uptime)));
    }
    if let Some(usage) = process.usage() {
        facts.push(format!("{:.1}%", usage.cpu_percent()));
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
        facts.push(exit_fact(exit.outcome()));
        facts.extend(ended_ago(exit, now));
    }
    facts
}

/// How a run ended in a word or two: `exit 1`, or the signal that ended it.
pub fn exit_fact(outcome: &ExitOutcome) -> String {
    match outcome {
        ExitOutcome::Succeeded => format!("{EXIT_WORD} {SUCCESS_STATUS}"),
        ExitOutcome::Failed(ExitCause::Status(status)) => format!("{EXIT_WORD} {status}"),
        ExitOutcome::Failed(ExitCause::Signal(signal)) => signal.to_string(),
        ExitOutcome::Failed(ExitCause::Unknown) => UNKNOWN_EXIT.to_string(),
    }
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

/// `text` after the mark of `kind`.
fn marked(kind: RowKind, text: &str) -> String {
    match kind.styled_mark() {
        Some(mark) => format!("{mark} {text}"),
        None => text.to_string(),
    }
}

/// The display width of the widest of `texts`.
fn widest(texts: impl Iterator<Item = String>) -> usize {
    texts
        .map(|text| UnicodeWidthStr::width(text.as_str()))
        .max()
        .unwrap_or(0)
}

/// The spaces that pad `text` to `width` display columns.
fn pad(text: &str, width: usize) -> String {
    " ".repeat(width.saturating_sub(UnicodeWidthStr::width(text)))
}
