//! The process tools: lifecycle, waiting, output, input and the stack
//! definition of one process.

use std::{io, path::Path, time::SystemTime};

use rmcp::{
    RoleServer, handler::server::wrapper::Parameters, service::RequestContext, tool, tool_router,
};
use serde_json::Value;

use super::{
    alerts_view::AlertsView,
    bounds::{CALL_BOUND, wait_bound},
    condition_arg::{ConditionArg, log_pattern},
    control_arg::ControlArg,
    control_view::ControlView,
    logs_view::LogsView,
    name_arg::NameArg,
    pending::{self, LineFilter, LineMatcher, LogQuery, OutputCursor, WaitOutcome, WaitSpec},
    process_add_arg::ProcessAddArg,
    process_fields::{invalid, process_name},
    process_logs_arg::ProcessLogsArg,
    process_set_arg::ProcessSetArg,
    process_wait_arg::ProcessWaitArg,
    progress::with_progress,
    rendering::{log_text, screen_text, tail_text},
    send_input_arg::SendInputArg,
    server::{
        MusterMcp, ToolResult, json, object, rendered, structured, unheard, with_alerts, with_field,
    },
    tool_error::ToolError,
    wait_view::WaitView,
};
use crate::{
    adapter::{
        bridge::WorkspaceRequest,
        client::{
            ActionReport, ClientError, DefinitionView, ProjectIdentityView, RemovedView,
            ServiceClient, Unwatched, process,
        },
        holder::HolderError,
        service::{ProcessAction, ServiceRequest},
    },
    domain::{
        config::{ConfigError, FieldEdit},
        project::Project,
        value::{ProcessName, ProcessRef},
    },
};

/// Lines `process_logs` returns when the call does not say.
const DEFAULT_LOG_LINES: usize = 100;
/// Most lines `process_logs` returns.
const MAX_LOG_LINES: usize = 2000;
/// Bytes `process_logs` returns when the call does not say.
const DEFAULT_LOG_BYTES: usize = 16 * 1024;
/// Most bytes `process_logs` returns.
const MAX_LOG_BYTES: usize = 64 * 1024;
/// Carriage return that submits a line typed into a terminal.
const ENTER: char = '\r';
/// The fields the process tools name in a refusal.
const UNTIL_FIELD: &str = "until";
const SINCE_FIELD: &str = "since";
const GREP_FIELD: &str = "grep";
const REGEX_FIELD: &str = "regex";
/// What `regex` needs beside it.
const REGEX_NEEDS_GREP: &str = "regex only together with grep";
/// What `grep` cannot be combined with.
const GREP_OR_ERRORS: &str = "grep or errors_only, not both";
/// Keys of the results built here.
const PROJECT: &str = "project";
const PROCESS: &str = "process";
const RUNNING: &str = "running";
const SCREEN: &str = "screen";
const SENT: &str = "sent";
const WAIT: &str = "wait";
/// How a removal whose report names no process is described.
const REMOVAL_UNREPORTED: &str = "a removal report naming no process";
/// How a control answer carrying neither a report nor a wait is described.
const CONTROL_UNREPORTED: &str = "a control answer with no report";
/// The form a grep regex is written in.
const GREP_REGEX_FORM: &str = "a valid regex";
/// The tool `process_set`'s refusal names.
const PROCESS_SET: &str = "process_set";

/// The process reference an agent typed as `name`.
///
/// # Errors
/// Returns the caller's mistake for a blank name.
fn reference(name: &str) -> Result<ProcessRef, ToolError> {
    ProcessRef::try_new(name.to_string()).map_err(|_| ToolError::InvalidName(name.to_string()))
}

/// The output cursor an agent typed as `raw`, refused as the field `field`.
///
/// # Errors
/// Returns the caller's mistake for a malformed cursor.
fn output_cursor(
    field: &'static str,
    raw: Option<&str>,
) -> Result<Option<OutputCursor>, ToolError> {
    raw.map(str::parse::<OutputCursor>)
        .transpose()
        .map_err(|reason| ToolError::InvalidField { field, reason })
}

/// The process of `project` an agent named `name`.
///
/// # Errors
/// Returns the caller's mistake for an unknown name, or a failure reaching
/// the service.
fn resolved(
    client: &ServiceClient,
    project: &Project,
    name: &ProcessRef,
) -> Result<ProcessName, ToolError> {
    Ok(process::resolve(
        &process::status(client, project)?,
        project,
        name,
    )?)
}

/// A wait's outcome as a result: flagged when it ended in a way the agent
/// must act on, with the run's last lines as its text.
///
/// # Errors
/// Returns an internal error if the view cannot be serialized.
fn waited(value: Value, outcome: &WaitOutcome, alerts: &AlertsView) -> ToolResult {
    let failed = outcome.status().is_failure();
    if failed && !outcome.tail().is_empty() {
        return rendered(tail_text(outcome), value, alerts, failed);
    }
    structured(with_alerts(value, alerts)?, failed)
}

/// The failure of reading the log at `log` with the log reader.
fn log_failure(log: &Path, error: HolderError) -> ToolError {
    let source = match error {
        HolderError::Io(source) => source,
        other => io::Error::other(other.to_string()),
    };
    ToolError::Client(process::log_error(log, source))
}

#[tool_router(router = process_router, vis = "pub(super)")]
impl MusterMcp {
    /// Starts a process.
    #[tool(
        description = "Start a process unless it is running. wait=true waits until it is ready \
            (its readiness check, or running when it has none); wait can also be a condition, \
            as in process_wait. Without wait, waits at most 45 seconds for it to settle. Returns \
            {project, process, outcome, reason, status, notices, wait_ended, alerts}, or with \
            wait {process, wait, alerts}. outcome is started, already-running, refused (with \
            reason), crashed, exited, not-started, still-starting or unsupervised. Flagged \
            isError when it failed, crashed or exited while waiting; a wait that ran out of time \
            is status waiting, not an error: continue with process_wait."
    )]
    pub(super) async fn process_start(
        &self,
        Parameters(arg): Parameters<ControlArg>,
        context: RequestContext<RoleServer>,
    ) -> ToolResult {
        with_progress(&context, self.control(arg, ProcessAction::Start)).await
    }

    /// Stops a process.
    #[tool(
        description = "Stop a process gracefully, without its restart policy respawning it. \
            Waits at most 45 seconds. Returns {project, process, outcome (stopped, \
            was-not-running, still-stopping), status, notices, wait_ended, alerts}. \
            still-stopping after the 45 seconds is not an error; check with stack_status."
    )]
    pub(super) async fn process_stop(
        &self,
        Parameters(arg): Parameters<NameArg>,
        context: RequestContext<RoleServer>,
    ) -> ToolResult {
        let arg = ControlArg::builder()
            .name(arg.name().clone())
            .project(arg.project().clone())
            .build();
        with_progress(&context, self.control(arg, ProcessAction::Stop)).await
    }

    /// Restarts a process.
    #[tool(
        description = "Restart a process, starting it if it was not running. This is the usual \
            call after changing code: pass wait=true to return once it is ready. Output from \
            the previous run never satisfies the wait. Same result as process_start, with \
            outcome restarted."
    )]
    pub(super) async fn process_restart(
        &self,
        Parameters(arg): Parameters<ControlArg>,
        context: RequestContext<RoleServer>,
    ) -> ToolResult {
        with_progress(&context, self.control(arg, ProcessAction::Restart)).await
    }

    /// Waits for a condition on a process.
    #[tool(
        description = "Block until a process's output contains a line matching until.log \
            (substring, or regex with regex=true), until.port accepts a TCP connection on \
            localhost, until.http answers below 500 (or with until.status), until.state is \
            reached, or until.ready=true when its readiness check passes. Default until: ready \
            when the process has a readiness check, else running. Only output after since \
            counts; without since, only output after this call. Waits at most timeout_s \
            (default 30, at most 45). Returns {process, status, until, match, cursor, \
            elapsed_ms, tail, hint, alerts}. status waiting is not an error: call again with \
            since=cursor. exited, crashed, stopped, not-running and ready-timeout are flagged \
            isError and include the last 30 lines in tail."
    )]
    pub(super) async fn process_wait(
        &self,
        Parameters(arg): Parameters<ProcessWaitArg>,
        context: RequestContext<RoleServer>,
    ) -> ToolResult {
        with_progress(&context, self.wait(arg)).await
    }

    /// Reads a process's log.
    #[tool(
        description = "Read a process's output log, with terminal escapes removed, whether or \
            not it is running. Without since, returns the last lines lines (default 100, at most \
            2000); with since (a cursor from any earlier result), returns only output after it. \
            grep keeps matching lines (regex=true for a regex); errors_only keeps error lines \
            with their stack traces. Capped at max_bytes (default 16384, at most 65536), keeping \
            the newest lines and saying how many were omitted. Returns {process, text, cursor, \
            from, run_start, lines_returned, omitted_lines, rotated_away_bytes, alerts}; pass \
            cursor as since next time to get only new output, or run_start to read only the \
            current run."
    )]
    pub(super) async fn process_logs(
        &self,
        Parameters(arg): Parameters<ProcessLogsArg>,
    ) -> ToolResult {
        let name = reference(arg.name())?;
        let since = output_cursor(SINCE_FIELD, arg.since().as_deref())?;
        let filter = log_filter(&arg)?;
        let query = LogQuery::builder()
            .since(since)
            .lines(
                arg.lines()
                    .unwrap_or(DEFAULT_LOG_LINES)
                    .clamp(1, MAX_LOG_LINES),
            )
            .filter(filter)
            .max_bytes(
                arg.max_bytes()
                    .unwrap_or(DEFAULT_LOG_BYTES)
                    .clamp(1, MAX_LOG_BYTES),
            )
            .build();
        let ((name, slice, run_start), alerts) = self
            .ask(arg.project().clone(), move |client, project| {
                let status = process::status(client, project)?;
                let name = process::resolve(&status, project, &name)?;
                let run_start = status
                    .processes()
                    .iter()
                    .find(|process| process.name() == &name)
                    .and_then(pending::run_start);
                let log = process::log_path(client, project, &name)?;
                let slice =
                    pending::read_log(&log, &query).map_err(|error| log_failure(&log, error))?;
                Ok((name, slice, run_start))
            })
            .await?;
        let text = slice.lines().join("\n");
        let view = LogsView::builder()
            .process(name.clone())
            .from(slice.from().to_string())
            .cursor(slice.cursor().to_string())
            .run_start(run_start.map(|cursor| cursor.to_string()))
            .lines_returned(slice.lines().len())
            .omitted_lines(slice.omitted_lines())
            .rotated_away_bytes(slice.rotated_away_bytes())
            .text(text)
            .build();
        rendered(log_text(&name, &slice), json(&view)?, &alerts, false)
    }

    /// Reads a process's visible screen.
    #[tool(
        description = "What a process's terminal shows right now, as text. Use it for \
            full-screen or interactive programs; use process_logs for ordinary output. Empty \
            when it has not started. Returns {process, screen, alerts}."
    )]
    pub(super) async fn process_screen(&self, Parameters(arg): Parameters<NameArg>) -> ToolResult {
        let name = reference(arg.name())?;
        let (read, alerts) = self
            .ask(arg.project().clone(), move |client, project| {
                process::logs(client, project, process::named(project, &name), true)
            })
            .await?;
        let screen = screen_text(&String::from_utf8_lossy(read.bytes()));
        let value = with_field(
            with_field(object(), PROCESS, json(read.name())?),
            SCREEN,
            Value::String(screen.clone()),
        );
        rendered(screen, value, &alerts, false)
    }

    /// Types text into a process's terminal.
    #[tool(
        description = "Type text into a process's terminal, as if a person typed it, to answer \
            a prompt or press a key in a watcher (for example \"r\" to reload). enter=true \
            submits the line. Returns {sent: true, alerts}. Flagged isError when the process is \
            not running."
    )]
    pub(super) async fn process_send(
        &self,
        Parameters(arg): Parameters<SendInputArg>,
    ) -> ToolResult {
        let name = reference(arg.name())?;
        let data = if arg.enter().unwrap_or(false) {
            format!("{}{ENTER}", arg.text())
        } else {
            arg.text().clone()
        };
        let ((), alerts) = self
            .ask(arg.project().clone(), move |client, project| {
                let name = resolved(client, project, &name)?;
                let request = ServiceRequest::Workspace {
                    project: project.root().clone(),
                    request: WorkspaceRequest::SendInput {
                        name: name.clone(),
                        data,
                    },
                };
                Ok(process::expect_ok(
                    client.send(&request)?,
                    project,
                    Some(&name),
                )?)
            })
            .await?;
        structured(
            with_alerts(with_field(object(), SENT, Value::Bool(true)), &alerts)?,
            false,
        )
    }

    /// Adds a process to a project's stack.
    #[tool(
        description = "Add a process to this project's stack, registering the folder as a \
            project if it is not one. Takes name, command (a shell command), and optionally \
            working_dir (relative to the project root), description, environment (an object of \
            variable names to values), autostart, restart (never|on-failure|always), \
            stop_signal (terminate|interrupt), stop_grace (such as 5s) and ready, a readiness \
            check such as {\"log\": \"Listening on\", \"timeout\": \"60s\"}, {\"port\": 3000, \
            \"timeout\": \"30s\"} or {\"http\": \"http://localhost:3000/health\", \"timeout\": \
            \"30s\"}; fields left out take the default-* settings that config_get shows. The \
            change is saved atomically and is visible to the human's muster ps at once. Does \
            not start it: call process_start. Returns {project, process: the stored \
            definition}. Flagged isError, naming the field, when the name is taken or a value \
            is invalid."
    )]
    pub(super) async fn process_add(
        &self,
        Parameters(arg): Parameters<ProcessAddArg>,
    ) -> ToolResult {
        let (project, spec) = self
            .blocking(move |deps| {
                let root = deps.target(arg.project().as_deref())?.root().clone();
                let draft = arg.draft(&root)?;
                if let Some(ready) = arg.ready() {
                    pending::store_ready(Some(&ready.check()?))?;
                }
                let (registration, spec) = deps.editor().add(&root, draft)?;
                Ok((registration.project().clone(), spec))
            })
            .await?;
        structured(
            with_field(
                with_field(
                    object(),
                    PROJECT,
                    json(&ProjectIdentityView::from(&project))?,
                ),
                PROCESS,
                json(&DefinitionView::from(&spec))?,
            ),
            false,
        )
    }

    /// Changes fields of a process in a project's stack.
    #[tool(
        description = "Change fields of a process in this project's stack; omitted fields keep \
            their value. Takes name and any of command, working_dir (null runs it in the root), \
            description (null removes it), environment_set (an object of variables to add or \
            replace), environment_unset (an array of names to remove), autostart, restart, \
            stop_signal, stop_grace and ready (as process_add takes it; null removes the \
            readiness check). A running process keeps its current run: restart it to apply the \
            change. Returns {project, process: the stored definition, running}. An unknown \
            name, an invalid value, or a call that changes nothing is flagged isError."
    )]
    pub(super) async fn process_set(
        &self,
        Parameters(arg): Parameters<ProcessSetArg>,
    ) -> ToolResult {
        let (project, spec, running) = self
            .blocking(move |deps| {
                let project = deps.project(arg.project().as_deref())?;
                let name = process_name(arg.name())?;
                let change = arg.change(project.root())?;
                let ready = arg.ready_edit()?;
                match &ready {
                    FieldEdit::Keep => {},
                    FieldEdit::Set(check) => pending::store_ready(Some(check))?,
                    FieldEdit::Clear => pending::store_ready(None)?,
                }
                if change.is_empty() {
                    return Err(ToolError::NothingToChange(PROCESS_SET));
                }
                let spec = deps.editor().change(project.root(), &name, &change)?;
                let running =
                    process::runs_now(deps.service().as_ref(), project.root(), spec.name());
                Ok((project, spec, running))
            })
            .await?;
        structured(
            with_field(
                with_field(
                    with_field(
                        object(),
                        PROJECT,
                        json(&ProjectIdentityView::from(&project))?,
                    ),
                    PROCESS,
                    json(&DefinitionView::from(&spec))?,
                ),
                RUNNING,
                Value::Bool(running),
            ),
            false,
        )
    }

    /// Removes a process from a project's stack.
    #[tool(
        description = "Remove a process from this project's stack, stopping it first if it \
            runs, waiting at most 45 seconds. Returns {project, name, stopped (it was running \
            and has ended), still_stopping (it had not ended when the wait ended; it still \
            ends), alerts}. An unknown name is flagged isError, listing the valid names."
    )]
    pub(super) async fn process_remove(&self, Parameters(arg): Parameters<NameArg>) -> ToolResult {
        let alerts = self.alerts.clone();
        let (project, removed, heard) = self
            .blocking(move |deps| {
                let project = deps.project(arg.project().as_deref())?;
                let typed = process_name(arg.name())?;
                let stack = deps.stacks().stack(project.root())?.ok_or_else(|| {
                    ConfigError::UnknownProject {
                        root: project.root().as_ref().clone(),
                    }
                })?;
                let name = stack.require(&typed)?.name().clone();
                let client = deps.service().connect()?;
                let before = process::status(&client, &project)?;
                let sent = SystemTime::now();
                deps.editor().remove(project.root(), &name)?;
                let names = [name];
                let report = process::settle_removed(
                    &client,
                    &project,
                    &before,
                    &names,
                    sent,
                    CALL_BOUND,
                    &mut Unwatched,
                )?;
                let removed =
                    report
                        .processes()
                        .first()
                        .map(RemovedView::from)
                        .ok_or(ToolError::Client(ClientError::UnexpectedAnswer(
                            REMOVAL_UNREPORTED,
                        )))?;
                let heard = unheard(&alerts, &client, &project);
                Ok((project, removed, heard))
            })
            .await?;
        structured(
            with_alerts(
                with_field(
                    json(&removed)?,
                    PROJECT,
                    json(&ProjectIdentityView::from(&project))?,
                ),
                &heard,
            )?,
            false,
        )
    }
}

impl MusterMcp {
    /// Applies `action` to the process `arg` names, then waits as it asks or
    /// reports what became of it once it settled.
    ///
    /// # Errors
    /// Returns the caller's mistake for a bad name or condition, or the
    /// fault of a service that cannot be reached.
    pub(super) async fn control(&self, arg: ControlArg, action: ProcessAction) -> ToolResult {
        let name = reference(arg.name())?;
        let bound = wait_bound(arg.timeout_s());
        let wait: Option<WaitSpec> = arg
            .wait()
            .as_ref()
            .map(|wait| wait.spec(bound))
            .transpose()?
            .flatten();
        let ((report, outcome), alerts) = self
            .ask(arg.project().clone(), move |client, project| {
                let answer: (Option<ActionReport>, Option<WaitOutcome>) = match &wait {
                    Some(spec) => {
                        let name = resolved(client, project, &name)?;
                        (
                            None,
                            Some(pending::control_and_wait(
                                client, project, &name, action, spec,
                            )?),
                        )
                    },
                    None => (
                        Some(process::control_within(
                            client,
                            project,
                            process::named::<ToolError>(project, &name),
                            action,
                            CALL_BOUND,
                            &mut Unwatched,
                        )?),
                        None,
                    ),
                };
                Ok(answer)
            })
            .await?;
        if let Some(outcome) = outcome {
            let value = with_field(
                with_field(object(), PROCESS, json(outcome.process())?),
                WAIT,
                json(&WaitView::from(&outcome))?,
            );
            return waited(value, &outcome, &alerts);
        }
        let report = report.ok_or(ToolError::Client(ClientError::UnexpectedAnswer(
            CONTROL_UNREPORTED,
        )))?;
        structured(
            with_alerts(json(&ControlView::from(&report))?, &alerts)?,
            report.failed(),
        )
    }

    /// What `process_wait` does, without its progress notifications.
    ///
    /// # Errors
    /// Returns the caller's mistake for a bad name, condition or cursor, or
    /// the fault of a service that cannot be reached.
    pub(super) async fn wait(&self, arg: ProcessWaitArg) -> ToolResult {
        let name = reference(arg.name())?;
        let spec = WaitSpec::builder()
            .until(
                arg.until()
                    .as_ref()
                    .map(|until: &ConditionArg| until.condition(UNTIL_FIELD))
                    .transpose()?,
            )
            .since(output_cursor(SINCE_FIELD, arg.since().as_deref())?)
            .timeout(wait_bound(arg.timeout_s()))
            .build();
        let (outcome, alerts) = self
            .ask(arg.project().clone(), move |client, project| {
                let name = resolved(client, project, &name)?;
                pending::wait(client, project, &name, &spec)
            })
            .await?;
        waited(json(&WaitView::from(&outcome))?, &outcome, &alerts)
    }
}

/// The lines `arg` keeps.
///
/// # Errors
/// Returns the caller's mistake for a regex without grep, grep together
/// with errors_only, or a pattern out of form.
fn log_filter(arg: &ProcessLogsArg) -> Result<LineFilter, ToolError> {
    let regex = arg.regex().unwrap_or(false);
    let errors_only = arg.errors_only().unwrap_or(false);
    match arg.grep() {
        Some(grep) if errors_only => Err(invalid(GREP_FIELD, GREP_OR_ERRORS, grep)),
        Some(grep) => {
            let pattern = log_pattern(GREP_FIELD, grep, regex)?;
            let matcher = LineMatcher::compile(&pattern)
                .map_err(|_| invalid(GREP_FIELD, GREP_REGEX_FORM, grep))?;
            Ok(LineFilter::Grep(matcher))
        },
        None if regex => Err(invalid(REGEX_FIELD, REGEX_NEEDS_GREP, "true")),
        None if errors_only => Ok(LineFilter::ErrorsOnly),
        None => Ok(LineFilter::All),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        adapter::mcp::test_support::{add, content, fixture, refusal, register, runtime},
        domain::{
            port::{ProjectStore, StackStore},
            value::ProcessName,
        },
    };

    /// `process_set` arguments read from the JSON an agent sends.
    fn set(json: serde_json::Value) -> Parameters<ProcessSetArg> {
        Parameters(serde_json::from_value(json).unwrap())
    }

    /// The process `name` as a tool argument.
    fn named(name: &str) -> Parameters<NameArg> {
        Parameters(NameArg::builder().name(name.to_string()).build())
    }

    /// The `process_logs` arguments an agent sent as `json`.
    fn logs(json: serde_json::Value) -> ProcessLogsArg {
        serde_json::from_value(json).unwrap()
    }

    /// `process_add` in a folder no project claims registers it, stores the
    /// definition with the default settings for what it left out, names the
    /// project, and tells a running service which project changed.
    #[test]
    fn process_add_registers_the_folder() {
        let fixture = fixture();
        assert!(fixture.store.project_at(&fixture.root).unwrap().is_none());

        let added = runtime().block_on(async {
            content(
                fixture
                    .server
                    .process_add(Parameters(
                        ProcessAddArg::builder()
                            .name("web".to_string())
                            .command("npm run dev".to_string())
                            .working_dir(Some("frontend".to_string()))
                            .environment(Some([("PORT".to_string(), "5173".to_string())].into()))
                            .restart(Some("on-failure".to_string()))
                            .build(),
                    ))
                    .await,
            )
        });
        assert_eq!(added[PROJECT]["name"], "shop");
        let definition = &added[PROCESS];
        assert_eq!(definition["name"], "web");
        assert_eq!(definition["working_dir"], "frontend");
        assert_eq!(definition["environment"]["PORT"], "5173");
        assert_eq!(definition["restart"], "on-failure");
        assert_eq!(definition["autostart"], false, "the seeded default");

        assert!(fixture.store.project_at(&fixture.root).unwrap().is_some());
        let stack = fixture
            .store
            .stack(&fixture.root)
            .unwrap()
            .expect("a stack");
        assert_eq!(stack.names(), vec![ProcessName::try_new("web").unwrap()]);
        assert_eq!(*fixture.notifier.stacks.lock().unwrap(), vec![
            fixture.root.clone()
        ]);
    }

    /// A taken name and an invalid field are the caller's mistakes; neither
    /// writes nor notifies.
    #[test]
    fn process_add_refuses_a_taken_name_and_an_invalid_field() {
        let fixture = fixture();
        register(&fixture);

        runtime().block_on(async {
            let taken = refusal(
                fixture
                    .server
                    .process_add(Parameters(add("web", "true")))
                    .await,
            );
            assert!(taken.contains("already exists"), "{taken}");

            let invalid = refusal(
                fixture
                    .server
                    .process_add(Parameters(
                        ProcessAddArg::builder()
                            .name("worker".to_string())
                            .command("bin/worker".to_string())
                            .restart(Some("sometimes".to_string()))
                            .build(),
                    ))
                    .await,
            );
            assert!(
                invalid.contains("restart") && invalid.contains("on-failure"),
                "{invalid}"
            );
        });

        let stack = fixture
            .store
            .stack(&fixture.root)
            .unwrap()
            .expect("a stack");
        assert_eq!(stack.names().len(), 1);
        assert_eq!(fixture.notifier.stacks.lock().unwrap().len(), 1);
    }

    /// A readiness check out of form is refused naming the ready field, and
    /// nothing is written.
    #[test]
    fn process_add_refuses_a_bad_readiness_check_before_writing() {
        let fixture = fixture();

        let refused = runtime().block_on(async {
            refusal(
                fixture
                    .server
                    .process_add(Parameters(
                        serde_json::from_value(serde_json::json!({
                            "name": "api",
                            "command": "npm start",
                            "ready": { "http": "http://example.com/", "timeout": "30s" },
                        }))
                        .unwrap(),
                    ))
                    .await,
            )
        });

        assert!(refused.contains("invalid ready"), "{refused}");
        assert!(fixture.store.project_at(&fixture.root).unwrap().is_none());
    }

    /// `process_set` changes only the fields it is given and reports the
    /// process as not running when no service has it loaded, asking the
    /// injected service rather than the user's.
    #[test]
    fn process_set_changes_the_given_fields() {
        let fixture = fixture();
        register(&fixture);

        let changed = runtime().block_on(async {
            content(
                fixture
                    .server
                    .process_set(set(serde_json::json!({
                        "name": "web",
                        "restart": "always",
                        "environment_set": { "PORT": "3000" },
                    })))
                    .await,
            )
        });
        assert_eq!(changed[PROCESS]["restart"], "always");
        assert_eq!(changed[PROCESS]["environment"]["PORT"], "3000");
        assert_eq!(changed[PROJECT]["name"], "shop");
        assert_eq!(changed[RUNNING], false);
        assert_eq!(fixture.notifier.stacks.lock().unwrap().len(), 2);
    }

    /// A `process_set` that changes nothing, or names no process, is the
    /// caller's mistake; an unknown name lists the valid ones.
    #[test]
    fn process_set_refuses_no_change_and_an_unknown_name() {
        let fixture = fixture();
        register(&fixture);

        runtime().block_on(async {
            let nothing = refusal(
                fixture
                    .server
                    .process_set(set(serde_json::json!({ "name": "web" })))
                    .await,
            );
            assert!(nothing.contains("changed nothing"), "{nothing}");
            let unknown = refusal(
                fixture
                    .server
                    .process_set(set(serde_json::json!({ "name": "wbe", "autostart": true })))
                    .await,
            );
            assert!(unknown.contains("'web'"), "{unknown}");
        });
    }

    /// `process_remove` of an unknown name lists the valid ones and removes
    /// nothing, before any service is reached.
    #[test]
    fn process_remove_refuses_an_unknown_name() {
        let fixture = fixture();
        register(&fixture);

        runtime().block_on(async {
            let unknown = refusal(fixture.server.process_remove(named("wbe")).await);
            assert!(unknown.contains("'web'"), "{unknown}");
        });

        let stack = fixture
            .store
            .stack(&fixture.root)
            .unwrap()
            .expect("a stack");
        assert_eq!(stack.names().len(), 1);
    }

    /// A blank process name, a malformed cursor and a bad condition are the
    /// caller's mistakes, refused before any service is reached.
    #[test]
    fn bad_arguments_are_refused_before_the_service() {
        let fixture = fixture();
        register(&fixture);

        runtime().block_on(async {
            refusal(fixture.server.process_screen(named("   ")).await);
            let cursor = refusal(
                fixture
                    .server
                    .wait(
                        serde_json::from_value(
                            serde_json::json!({ "name": "web", "since": "18234" }),
                        )
                        .unwrap(),
                    )
                    .await,
            );
            assert!(
                cursor.contains("since") && cursor.contains("o:"),
                "{cursor}"
            );
            let until = refusal(
                fixture
                    .server
                    .wait(
                        serde_json::from_value(serde_json::json!({
                            "name": "web",
                            "until": { "http": "http://169.254.169.254/" },
                        }))
                        .unwrap(),
                    )
                    .await,
            );
            assert!(until.contains("invalid until"), "{until}");
            let wait = refusal(
                fixture
                    .server
                    .control(
                        serde_json::from_value(
                            serde_json::json!({ "name": "web", "wait": { "port": 0 } }),
                        )
                        .unwrap(),
                        ProcessAction::Start,
                    )
                    .await,
            );
            assert!(wait.contains("invalid wait"), "{wait}");
        });
    }

    /// The log filter refuses regex without grep and grep with errors_only,
    /// and reads each filter otherwise.
    #[test]
    fn the_log_filter_reads_one_filter() {
        assert!(log_filter(&logs(serde_json::json!({ "name": "w", "regex": true }))).is_err());
        assert!(
            log_filter(&logs(
                serde_json::json!({ "name": "w", "grep": "x", "errors_only": true })
            ))
            .is_err()
        );
        assert!(matches!(
            log_filter(&logs(
                serde_json::json!({ "name": "w", "errors_only": true })
            ))
            .unwrap(),
            LineFilter::ErrorsOnly
        ));
        assert!(matches!(
            log_filter(&logs(serde_json::json!({ "name": "w", "grep": "GET" }))).unwrap(),
            LineFilter::Grep(_)
        ));
        assert!(matches!(
            log_filter(&logs(serde_json::json!({ "name": "w" }))).unwrap(),
            LineFilter::All
        ));
    }

    /// A tool reaching the service in a fixture never reaches the user's:
    /// the injected service refuses, as an unreachable one would.
    #[test]
    fn the_service_is_never_the_users() {
        let fixture = fixture();
        register(&fixture);

        let error = runtime()
            .block_on(async { fixture.server.process_screen(named("web")).await })
            .unwrap_err()
            .answer()
            .unwrap_err();
        assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
        assert_eq!(
            error.data.expect("the failure rides along")["error"]["kind"],
            "service-unavailable"
        );
    }
}
