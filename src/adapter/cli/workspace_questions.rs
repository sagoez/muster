//! The questions `muster init` asks at a terminal: each process's name,
//! kind, command, autostart and restart policy, one process after another.

use std::collections::HashSet;

use cliclack::{confirm, input, select};

use super::{error::CliError, presenter::asked, process_answer::ProcessAnswer};
use crate::{
    adapter::config::starter_workspace,
    domain::{
        config::{ProcessSpec, WorkspaceConfig},
        process::{ProcessKind, RestartPolicy},
        value::{CommandLine, ProcessName},
    },
};

/// Asks for a process's name.
const NAME_PROMPT: &str = "Process name";
/// Refuses a name that is blank.
const NAME_BLANK: &str = "a process needs a name";
/// Refuses a name already given.
const NAME_TAKEN: &str = "another process already has that name";
/// Asks for a process's kind.
const KIND_PROMPT: &str = "Kind";
/// What each kind is for.
const TERMINAL_HINT: &str = "an interactive shell";
const COMMAND_HINT: &str = "a dev server, watcher or worker";
const AGENT_HINT: &str = "a CLI coding agent";
/// Asks for a process's command.
const COMMAND_PROMPT: &str = "Command";
/// Shown in an empty terminal command.
const SHELL_PLACEHOLDER: &str = "empty runs your login shell";
/// Refuses a blank command where one is needed.
const COMMAND_BLANK: &str = "this kind needs a command";
/// Asks whether `muster up` starts it.
const AUTOSTART_PROMPT: &str = "Start it with `muster up`?";
/// Asks how it restarts.
const RESTART_PROMPT: &str = "Restart it";
/// What each restart policy does.
const NEVER_LABEL: &str = "never";
const ON_FAILURE_LABEL: &str = "when it fails";
const ALWAYS_LABEL: &str = "whenever it exits";
/// Asks whether to add another process.
const ANOTHER_PROMPT: &str = "Add another process?";

/// Asks for the project's processes until the person adds no more, starting
/// from the starter's terminal as the first suggestion, and the workspace
/// they make.
///
/// # Errors
/// Returns [`CliError::Cancelled`] when the person cancels a question, a
/// [`CliError::Terminal`] when the terminal fails, or the
/// [`CliError::Config`] of a workspace that does not validate.
pub(super) fn ask_workspace() -> Result<WorkspaceConfig, CliError> {
    let starter = starter_workspace();
    let suggested = starter.terminals().first();
    let mut answers: Vec<ProcessAnswer> = Vec::new();
    loop {
        let first = answers.is_empty();
        let taken: HashSet<String> = answers
            .iter()
            .map(|answer| answer.spec().name().key())
            .collect();
        let answer = ask_process(taken, suggested.filter(|_| first))?;
        answers.push(answer);
        if !asked(|| confirm(ANOTHER_PROMPT).initial_value(false).interact())? {
            break;
        }
    }
    let section = |kind: ProcessKind| {
        answers
            .iter()
            .filter(|answer| answer.kind() == kind)
            .map(|answer| answer.spec().clone())
            .collect::<Vec<_>>()
    };
    let workspace = WorkspaceConfig::builder()
        .agents(section(ProcessKind::Agent))
        .terminals(section(ProcessKind::Terminal))
        .commands(section(ProcessKind::Command))
        .build();
    workspace.validate()?;
    Ok(workspace)
}

/// Asks for one process whose name is none of `taken`, offering `suggested`'s
/// name and autostart as the answers to accept.
///
/// # Errors
/// Returns [`CliError::Cancelled`] or a [`CliError::Terminal`].
fn ask_process(
    taken: HashSet<String>,
    suggested: Option<&ProcessSpec>,
) -> Result<ProcessAnswer, CliError> {
    let mut name_prompt = input(NAME_PROMPT).validate(move |typed: &String| {
        match ProcessName::try_new(typed.clone()) {
            Err(_) => Err(NAME_BLANK),
            Ok(name) if taken.contains(&name.key()) => Err(NAME_TAKEN),
            Ok(_) => Ok(()),
        }
    });
    if let Some(spec) = suggested {
        name_prompt = name_prompt.default_input(spec.name().as_ref());
    }
    let typed: String = asked(|| name_prompt.interact())?;
    let name = ProcessName::try_new(typed.clone()).map_err(|_| CliError::InvalidName(typed))?;
    let kind = asked(|| {
        select(KIND_PROMPT)
            .item(ProcessKind::Terminal, ProcessKind::Terminal, TERMINAL_HINT)
            .item(ProcessKind::Command, ProcessKind::Command, COMMAND_HINT)
            .item(ProcessKind::Agent, ProcessKind::Agent, AGENT_HINT)
            .interact()
    })?;
    let command = ask_command(kind)?;
    let autostart = asked(|| {
        confirm(AUTOSTART_PROMPT)
            .initial_value(suggested.is_none_or(|spec| *spec.autostart()))
            .interact()
    })?;
    let restart = asked(|| {
        select(RESTART_PROMPT)
            .item(RestartPolicy::Never, NEVER_LABEL, "")
            .item(RestartPolicy::OnFailure, ON_FAILURE_LABEL, "")
            .item(RestartPolicy::Always, ALWAYS_LABEL, "")
            .interact()
    })?;
    let spec = ProcessSpec::builder()
        .name(name)
        .command(command)
        .restart(Some(restart))
        .autostart(autostart)
        .build();
    Ok(ProcessAnswer::builder().kind(kind).spec(spec).build())
}

/// Asks for the command a process of `kind` runs: optional for a terminal,
/// which runs the login shell without one, required otherwise.
///
/// # Errors
/// Returns [`CliError::Cancelled`] or a [`CliError::Terminal`].
fn ask_command(kind: ProcessKind) -> Result<Option<CommandLine>, CliError> {
    let optional = kind == ProcessKind::Terminal;
    let mut prompt = input(COMMAND_PROMPT)
        .required(!optional)
        .validate(move |typed: &String| {
            if optional || CommandLine::try_new(typed.clone()).is_ok() {
                Ok(())
            } else {
                Err(COMMAND_BLANK)
            }
        });
    if optional {
        prompt = prompt.placeholder(SHELL_PLACEHOLDER);
    }
    let typed: String = asked(|| prompt.interact())?;
    Ok(CommandLine::try_new(typed).ok())
}
