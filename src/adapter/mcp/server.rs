use std::{
    path::Path,
    time::{Duration, SystemTime},
};

use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::stdio,
};
use serde::Serialize;
use tokio::task;

use super::{
    config_get_arg::ConfigGetArg,
    config_set_arg::ConfigSetArg,
    key_arg::KeyArg,
    kv_set_arg::KvSetArg,
    list_arg::ListArg,
    mcp_deps::McpDeps,
    mcp_error::McpError,
    name_arg::NameArg,
    process_add_arg::ProcessAddArg,
    process_fields::{KEY_FIELD, choice, process_name},
    process_set_arg::ProcessSetArg,
    scratchpad_entry::ScratchpadEntry,
    scratchpad_set_arg::ScratchpadSetArg,
    send_input_arg::SendInputArg,
    todo_add_arg::TodoAddArg,
    todo_id_arg::TodoIdArg,
    todo_status_arg::TodoStatusArg,
    tool_error::ToolError,
    tool_failure::ToolFailure,
};
use crate::{
    adapter::{
        bridge::WorkspaceRequest,
        client::{
            ActionReport, ClientError, DefinitionView, ProjectView, RemovedView, ServiceClient,
            SettingView, SettingsView, Unwatched, process,
        },
        service::{ProcessAction, ServiceRequest},
    },
    domain::{
        config::{ConfigError, SettingChange, SettingKey},
        coordination::{KvKey, ScratchpadKey, TodoId, TodoStatus, TodoTitle},
        project::Project,
        value::{ProcessRef, ProjectRoot, ValueParseError},
    },
};

/// Server name reported to connecting agents.
const MCP_SERVER_NAME: &str = "muster-coordination";
/// Key muster's server is registered under in an agent's MCP config; it
/// namespaces the tools an agent sees (e.g. `muster` -> `scratchpad_set`).
const MCP_CONNECTION_KEY: &str = "muster";
/// Subcommand an agent invokes to start the server.
const MCP_SUBCOMMAND: &str = "mcp";
/// Flag that names the project the server serves.
const MCP_PROJECT_FLAG: &str = "--project";
/// Server version reported to connecting agents.
const MCP_SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Guidance shown to agents that connect to the server.
const MCP_INSTRUCTIONS: &str = "muster runs this project's dev stack (servers, workers, \
    watchers) and holds coordination state that agents and the human share. The process_* \
    tools read and drive the live processes by name through the muster service, which starts \
    on demand. process_add, process_set and process_remove edit the project's stack, which the \
    human's `muster ps` shows at once; process_add registers this folder as a project when it \
    is not one. config_get and config_set read and change the machine's settings. Use \
    scratchpads to pass context and feedback, todos to track work, and key-values for small \
    shared facts (a chosen port, a build id). A mistake in a call, such as an unknown name, \
    comes back as a result flagged isError whose text says what to do.";
/// Entries a list tool returns when the caller does not ask for a size.
const DEFAULT_LIST_LIMIT: usize = 50;
/// Hard cap on a list tool's page, so one call cannot flood the caller's context.
const MAX_LIST_LIMIT: usize = 200;
/// Longest a lifecycle tool waits for its process to settle, under the 60
/// seconds many MCP clients give a call.
const SETTLE_BOUND: Duration = Duration::from_secs(45);
/// How a removal whose report names no process is described.
const REMOVAL_UNREPORTED: &str = "a removal report naming no process";
/// Acknowledgement of input written to a process's terminal.
const SENT: &str = "sent";
/// Carriage return that submits a line typed into a terminal.
const ENTER: char = '\r';
/// Key of the list in `scratchpad_list`'s result.
const SCRATCHPADS: &str = "scratchpads";
/// Key of the list in `todo_list`'s result.
const TODOS: &str = "todos";
/// Key of the list in `kv_list`'s result.
const ENTRIES: &str = "entries";
/// Key of the definition in `process_set`'s result.
const PROCESS: &str = "process";
/// Key of whether the process runs in `process_set`'s result.
const RUNNING: &str = "running";
/// The `status` field of `todo_set_status`.
const STATUS_FIELD: &str = "status";
/// The statuses `todo_set_status` accepts.
const STATUS_FORM: &str = "pending, in-progress or done";

/// What a tool answers: its result, or why it has none.
type ToolResult = Result<CallToolResult, ToolFailure>;

/// Builds the pretty-printed JSON snippet an agent pastes into its MCP-servers
/// config to launch muster's server for the project at `root`. `executable` is
/// the muster binary path, so the entry works even when muster is not on `PATH`.
pub fn connection_config(executable: &Path, root: &ProjectRoot) -> String {
    let value = serde_json::json!({
        "mcpServers": {
            MCP_CONNECTION_KEY: {
                "command": executable.display().to_string(),
                "args": [
                    MCP_SUBCOMMAND,
                    MCP_PROJECT_FLAG,
                    root.as_ref().display().to_string(),
                ],
            },
        },
    });
    serde_json::to_string_pretty(&value).unwrap_or_default()
}

/// Serves the project of `deps` as an MCP server over stdio, blocking until
/// the client disconnects. Intended to run as a child process an agent
/// spawns, so the OS process boundary is the trust boundary; no port and no
/// token are exposed.
///
/// # Errors
/// Returns [`McpError`] if the server cannot be initialized or stops abnormally.
pub async fn serve_stdio(deps: McpDeps) -> Result<(), McpError> {
    let service = CoordinationMcp::new(deps)
        .serve(stdio())
        .await
        .map_err(|error| McpError::Serve(error.to_string()))?;
    service
        .waiting()
        .await
        .map_err(|error| McpError::Wait(error.to_string()))?;
    Ok(())
}

/// Resolves a requested page size to the allowed range.
fn page_size(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_LIST_LIMIT).clamp(1, MAX_LIST_LIMIT)
}

/// MCP driving adapter exposing one project's stack, processes, settings and
/// coordination state as tools. Coordination writes are attributed to the
/// author in its deps - the agent this server was launched for.
pub struct CoordinationMcp {
    deps: McpDeps,
}

/// The process reference an agent typed as `name`.
///
/// # Errors
/// Returns the caller's mistake for a blank name.
fn reference(name: &str) -> Result<ProcessRef, ToolFailure> {
    ProcessRef::try_new(name.to_string())
        .map_err(|_| ToolError::InvalidName(name.to_string()).into())
}

/// `report` as a tool result, flagged as an error when a process did not end
/// up as asked so the agent cannot read it as a success.
fn action_result(report: &ActionReport) -> ToolResult {
    let value = serde_json::to_value(report)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    Ok(if report.failed() {
        CallToolResult::structured_error(value)
    } else {
        CallToolResult::structured(value)
    })
}

/// Types `data` into the terminal of the process `name` of `project`.
///
/// # Errors
/// Returns a [`ClientError`] when the name matches no process, the process
/// is not running, or the service cannot be reached.
fn send_input(
    client: &ServiceClient,
    project: &Project,
    name: &ProcessRef,
    data: String,
) -> Result<(), ClientError> {
    let name = process::resolve(&process::status(client, project)?, project, name)?;
    let request = ServiceRequest::Workspace {
        project: project.root().clone(),
        request: WorkspaceRequest::SendInput {
            name: name.clone(),
            data,
        },
    };
    process::expect_ok(client.send(&request)?, project, Some(&name))
}

/// A successful tool result carrying a single text block.
fn text(body: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(body)])
}

/// A successful tool result carrying structured JSON (with a text fallback that
/// rmcp fills in), so an agent gets typed data rather than a prose blob.
///
/// # Errors
/// Returns an internal error if the value cannot be serialized, which cannot
/// happen for the entities the tools return.
fn structured<T: Serialize>(value: &T) -> ToolResult {
    serde_json::to_value(value)
        .map(CallToolResult::structured)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None).into())
}

/// `items` as a structured result holding them under `key`, since a
/// structured result is always an object.
///
/// # Errors
/// Returns an internal error if the items cannot be serialized.
fn listing<T: Serialize>(key: &str, items: &[T]) -> ToolResult {
    structured(&serde_json::json!({ key: items }))
}

/// The setting key an agent typed as `raw`.
///
/// # Errors
/// Returns the caller's mistake for a key that names no setting, listing
/// every key.
fn setting_key(raw: &str) -> Result<SettingKey, ToolError> {
    choice(KEY_FIELD, raw)
}

#[tool_router]
impl CoordinationMcp {
    /// Builds the server over `deps`.
    pub fn new(deps: McpDeps) -> Self {
        Self { deps }
    }

    /// Lists every scratchpad in this project as structured objects.
    #[tool(
        description = "List the shared scratchpad notes in this project (other agents and the \
            human read and write these). Takes limit (default 50, at most 200). Returns \
            {scratchpads: [{key, author, updated_at (Unix seconds)}]}, an index without bodies \
            so a project of long notes cannot flood you. Read a body with scratchpad_get."
    )]
    async fn scratchpad_list(&self, Parameters(arg): Parameters<ListArg>) -> ToolResult {
        let notes = self.deps.coordination().scratchpads(self.deps.root())?;
        let page: Vec<ScratchpadEntry> = notes
            .iter()
            .take(page_size(arg.limit()))
            .map(|note| {
                ScratchpadEntry::builder()
                    .key(note.key().as_ref().to_string())
                    .author(note.author().clone())
                    .updated_at(*note.updated_at())
                    .build()
            })
            .collect();
        listing(SCRATCHPADS, &page)
    }

    /// Reads one scratchpad.
    #[tool(
        description = "Read one shared scratchpad note. Takes its key. Returns {key, body, \
            author, updated_at (Unix seconds)}. A result flagged isError says no note has that key."
    )]
    async fn scratchpad_get(&self, Parameters(arg): Parameters<KeyArg>) -> ToolResult {
        let key = scratchpad_key(arg.key())?;
        match self
            .deps
            .coordination()
            .scratchpad(self.deps.root(), &key)?
        {
            Some(note) => structured(&note),
            None => Err(ToolError::UnknownNote(key.as_ref().to_string()).into()),
        }
    }

    /// Creates or replaces a scratchpad.
    #[tool(
        description = "Create or replace a shared scratchpad note, visible to other agents and \
            the human; use it to pass context, plans or feedback. Takes key and a markdown body. \
            Returns \"saved '<key>'\"."
    )]
    async fn scratchpad_set(&self, Parameters(arg): Parameters<ScratchpadSetArg>) -> ToolResult {
        let key = scratchpad_key(arg.key())?;
        self.deps.coordination().set_scratchpad(
            self.deps.root(),
            self.deps.author(),
            &key,
            arg.body(),
        )?;
        Ok(text(format!("saved '{}'", key.as_ref())))
    }

    /// Deletes a scratchpad.
    #[tool(
        description = "Delete a scratchpad note. Takes its key. Returns \"deleted '<key>'\", or \
            \"no scratchpad '<key>'\" when there was none."
    )]
    async fn scratchpad_delete(&self, Parameters(arg): Parameters<KeyArg>) -> ToolResult {
        let key = scratchpad_key(arg.key())?;
        let removed = self
            .deps
            .coordination()
            .delete_scratchpad(self.deps.root(), &key)?;
        Ok(text(if removed {
            format!("deleted '{}'", key.as_ref())
        } else {
            format!("no scratchpad '{}'", key.as_ref())
        }))
    }

    /// Lists every todo as structured objects, including dependencies.
    #[tool(
        description = "List the shared todos in this project. Takes limit (default 50, at most \
            200). Returns {todos: [{id, title, status (pending|in-progress|done), deps (ids this \
            todo depends on), author, updated_at (Unix seconds)}]}."
    )]
    async fn todo_list(&self, Parameters(arg): Parameters<ListArg>) -> ToolResult {
        let todos = self.deps.coordination().todos(self.deps.root())?;
        let page: Vec<_> = todos.into_iter().take(page_size(arg.limit())).collect();
        listing(TODOS, &page)
    }

    /// Reads one todo, including its dependencies.
    #[tool(
        description = "Read one todo. Takes its id. Returns {id, title, status, deps, author, \
            updated_at}. A result flagged isError says no todo has that id."
    )]
    async fn todo_get(&self, Parameters(arg): Parameters<TodoIdArg>) -> ToolResult {
        let id = todo_id(arg.id())?;
        match self.deps.coordination().todo(self.deps.root(), &id)? {
            Some(todo) => structured(&todo),
            None => Err(ToolError::UnknownTodo(id.as_ref().to_string()).into()),
        }
    }

    /// Adds a new pending todo, returning the created todo.
    #[tool(
        description = "Add a new shared, pending todo. Takes title and deps, the ids of todos \
            this one depends on (optional). Returns the created todo {id, title, status, deps, \
            author, updated_at}."
    )]
    async fn todo_add(&self, Parameters(arg): Parameters<TodoAddArg>) -> ToolResult {
        let title = TodoTitle::try_new(arg.title().clone())
            .map_err(|_| ToolError::InvalidTodoTitle(arg.title().clone()))?;
        let deps = arg
            .deps()
            .iter()
            .map(|dep| todo_id(dep))
            .collect::<Result<Vec<_>, _>>()?;
        let created = self.deps.coordination().add_todo(
            self.deps.root(),
            self.deps.author(),
            &title,
            &deps,
        )?;
        structured(&created)
    }

    /// Sets a todo's lifecycle status.
    #[tool(
        description = "Set a todo's status. Takes its id and status (pending|in-progress|done). \
            Returns \"<id> is <status>\". A result flagged isError says the id or status is wrong."
    )]
    async fn todo_set_status(&self, Parameters(arg): Parameters<TodoStatusArg>) -> ToolResult {
        let id = todo_id(arg.id())?;
        let status = arg
            .status()
            .parse::<TodoStatus>()
            .map_err(|_| ToolError::InvalidField {
                field: STATUS_FIELD,
                reason: ValueParseError::builder()
                    .expected(STATUS_FORM)
                    .found(arg.status().as_str())
                    .build(),
            })?;
        if self.deps.coordination().set_todo_status(
            self.deps.root(),
            self.deps.author(),
            &id,
            status,
        )? {
            Ok(text(format!("{id} is {status}")))
        } else {
            Err(ToolError::UnknownTodo(id.as_ref().to_string()).into())
        }
    }

    /// Deletes a todo.
    #[tool(
        description = "Delete a todo. Takes its id. Returns \"deleted '<id>'\", or \"no todo \
            '<id>'\" when there was none."
    )]
    async fn todo_delete(&self, Parameters(arg): Parameters<TodoIdArg>) -> ToolResult {
        let id = todo_id(arg.id())?;
        let removed = self
            .deps
            .coordination()
            .delete_todo(self.deps.root(), &id)?;
        Ok(text(if removed {
            format!("deleted '{id}'")
        } else {
            format!("no todo '{id}'")
        }))
    }

    /// Lists every key-value entry as structured objects.
    #[tool(
        description = "List the shared key-value entries in this project. Takes limit (default \
            50, at most 200). Returns {entries: [{key, value, author, updated_at (Unix seconds)}]}."
    )]
    async fn kv_list(&self, Parameters(arg): Parameters<ListArg>) -> ToolResult {
        let entries = self.deps.coordination().values(self.deps.root())?;
        let page: Vec<_> = entries.into_iter().take(page_size(arg.limit())).collect();
        listing(ENTRIES, &page)
    }

    /// Reads one key-value entry.
    #[tool(
        description = "Read one shared key-value entry. Takes its key. Returns {key, value, \
            author, updated_at}. A result flagged isError says no entry has that key."
    )]
    async fn kv_get(&self, Parameters(arg): Parameters<KeyArg>) -> ToolResult {
        let key = kv_key(arg.key())?;
        match self.deps.coordination().value(self.deps.root(), &key)? {
            Some(entry) => structured(&entry),
            None => Err(ToolError::UnknownValue(key.as_ref().to_string()).into()),
        }
    }

    /// Creates or replaces a key-value entry.
    #[tool(
        description = "Create or replace a shared key-value entry, for small facts other agents \
            need: a chosen port, a build id, a flag. Takes key and value. Returns \"saved '<key>'\"."
    )]
    async fn kv_set(&self, Parameters(arg): Parameters<KvSetArg>) -> ToolResult {
        let key = kv_key(arg.key())?;
        self.deps.coordination().set_value(
            self.deps.root(),
            self.deps.author(),
            &key,
            arg.value(),
        )?;
        Ok(text(format!("saved '{}'", key.as_ref())))
    }

    /// Deletes a key-value entry.
    #[tool(
        description = "Delete a key-value entry. Takes its key. Returns \"deleted '<key>'\", or \
            \"no value '<key>'\" when there was none."
    )]
    async fn kv_delete(&self, Parameters(arg): Parameters<KeyArg>) -> ToolResult {
        let key = kv_key(arg.key())?;
        let removed = self
            .deps
            .coordination()
            .delete_value(self.deps.root(), &key)?;
        Ok(text(if removed {
            format!("deleted '{}'", key.as_ref())
        } else {
            format!("no value '{}'", key.as_ref())
        }))
    }

    /// Lists this project's processes.
    #[tool(
        description = "List this project's processes in stack order. Takes no arguments. \
            Returns {project, processes}, where each process is {name, state \
            (pending|starting|running|paused|stopping|exited|crashed|restarting|unsupervised), \
            pid, uptime (whole seconds), restarts, cpu_percent, memory_bytes, attached (how many \
            clients are attached to type into its terminal), last_exit ({status: exit code or \
            null, signal: the signal that ended it by its lowercase POSIX name such as sigterm \
            or sigkill, signal-<number> for one with no POSIX name, or null}, while it is not \
            running), stop_grace (whole seconds a stop waits before it kills)}. A null field has \
            no value right now. Use a process's name with the other process_* tools."
    )]
    async fn process_list(&self) -> ToolResult {
        let status = self
            .ask(|client, project| Ok(process::status(client, project)?))
            .await?;
        structured(&ProjectView::from(&status))
    }

    /// Reads the visible screen of a process's terminal.
    #[tool(
        description = "Read what a process's terminal shows right now - what a dev server or \
            test run last printed. Takes the process's name. Returns the screen as text without \
            its trailing blank lines, empty when the process has not started. An unknown name is \
            a result flagged isError that lists the valid names."
    )]
    async fn process_output(&self, Parameters(arg): Parameters<NameArg>) -> ToolResult {
        let name = reference(arg.name())?;
        let screen = self
            .ask(move |client, project| {
                process::logs(client, project, process::named(project, &name), true)
            })
            .await?;
        Ok(text(String::from_utf8_lossy(screen.bytes()).trim_end()))
    }

    /// Starts a process.
    #[tool(
        description = "Start a process unless it is already running. Takes the process's name. \
            Waits for it to settle, at most 45 seconds, then returns {project, action, processes: \
            [{name, outcome, reason, status}], notices, other_notices, wait_ended}: status is its \
            process_list entry now, notices explain a failure, and wait_ended is settled, \
            timed-out (muster's own wait for this action expired) or cut-short (the 45 second \
            bound expired first). outcome is one of started, already-running, exited (it ran and \
            exited), crashed, unsupervised (an earlier run may still be running, so it was not \
            started), not-started, still-starting or still-stopping (it had not settled when the \
            wait ended), or gone. The result is flagged isError when the outcome is anything but \
            started or already-running, except that still-starting or still-stopping after a \
            cut-short wait is not an error: the process is on its way, so follow it with \
            process_list."
    )]
    async fn process_start(&self, Parameters(arg): Parameters<NameArg>) -> ToolResult {
        self.control(arg.name(), ProcessAction::Start).await
    }

    /// Stops a process.
    #[tool(
        description = "Stop a process without its restart policy respawning it. Takes the \
            process's name. The stop is graceful: it waits out the process's stop grace, and \
            this tool waits at most 45 seconds in all. Returns the same report as process_start, \
            whose outcome is one of stopped, was-not-running, still-stopping (it had not exited \
            when the wait ended) or still-running. The result is flagged isError when the \
            outcome is anything but stopped or was-not-running, except that still-stopping after \
            a cut-short wait is not an error: follow it with process_list."
    )]
    async fn process_stop(&self, Parameters(arg): Parameters<NameArg>) -> ToolResult {
        self.control(arg.name(), ProcessAction::Stop).await
    }

    /// Restarts a process.
    #[tool(
        description = "Restart a process, whatever its restart policy, starting it if it was not \
            running. Takes the process's name. Waits for it to settle, at most 45 seconds, then \
            returns the same report as process_start, whose outcome is one of restarted, \
            still-running (the old run did not end), exited, crashed, unsupervised, not-started, \
            still-starting or still-stopping (it had not settled when the wait ended), or gone. \
            The result is flagged isError when the outcome is anything but restarted, except \
            that still-starting or still-stopping after a cut-short wait is not an error: follow \
            it with process_list."
    )]
    async fn process_restart(&self, Parameters(arg): Parameters<NameArg>) -> ToolResult {
        self.control(arg.name(), ProcessAction::Restart).await
    }

    /// Types text into a process's terminal.
    #[tool(
        description = "Type text into a process's terminal as if a person typed it - to answer \
            a prompt, run a command in a shell, or unblock a waiting process. Takes the process's \
            name, the text, and enter (default false); enter=true submits the line. Returns \
            \"sent\". A process that is not running, or an unknown name, is a result flagged \
            isError that says what to do."
    )]
    async fn process_send(&self, Parameters(arg): Parameters<SendInputArg>) -> ToolResult {
        let name = reference(arg.name())?;
        let data = if arg.enter() {
            format!("{}{ENTER}", arg.text())
        } else {
            arg.text().clone()
        };
        self.ask(move |client, project| Ok(send_input(client, project, &name, data)?))
            .await?;
        Ok(text(SENT))
    }

    /// Adds a process to this project's stack.
    #[tool(
        description = "Add a process to this project's stack, registering this folder as a \
            project if it is not one. Takes name, command (a shell command), and optionally \
            working_dir (relative to the project root), description, environment (an object of \
            variable names to values), autostart (whether muster up starts it), restart \
            (never|on-failure|always), stop_signal (terminate|interrupt) and stop_grace (a \
            duration such as 5s); fields left out take the default-* settings that config_get \
            shows. It does not start the process: call process_start. Returns the stored \
            definition {name, command, working_dir, description, environment, autostart, \
            restart, stop: {signal, grace (seconds)}}. A taken name or an invalid value is a \
            result flagged isError that names the field."
    )]
    async fn process_add(&self, Parameters(arg): Parameters<ProcessAddArg>) -> ToolResult {
        let (_, spec) = self
            .blocking(move |deps| {
                let draft = arg.draft(deps.root())?;
                Ok(deps.editor().add(deps.root(), draft)?)
            })
            .await?;
        structured(&DefinitionView::from(&spec))
    }

    /// Changes fields of a process in this project's stack.
    #[tool(
        description = "Change fields of a process in this project's stack; fields left out keep \
            their value. Takes name and any of command, working_dir (relative to the project \
            root, or null to run it in the root), description (null removes it), \
            environment_set (an object of variables to add or replace), environment_unset (an \
            array of names to remove), autostart, restart, stop_signal and stop_grace. A running \
            process keeps its current run: call process_restart to apply the change. Returns \
            {process: the stored definition, as process_add returns it, running}. An unknown \
            name, an invalid value, or a call that changes nothing is a result flagged isError."
    )]
    async fn process_set(&self, Parameters(arg): Parameters<ProcessSetArg>) -> ToolResult {
        let (spec, running) = self
            .blocking(move |deps| {
                let name = process_name(arg.name())?;
                let change = arg.change(deps.root())?;
                if change.is_empty() {
                    return Err(ToolError::NothingToChange);
                }
                let spec = deps.editor().change(deps.root(), &name, &change)?;
                let running = process::runs_now(deps.root(), spec.name());
                Ok((spec, running))
            })
            .await?;
        structured(&serde_json::json!({
            PROCESS: DefinitionView::from(&spec),
            RUNNING: running,
        }))
    }

    /// Removes a process from this project's stack.
    #[tool(
        description = "Remove a process from this project's stack, stopping it first when it \
            runs. Takes the process's name. It stops through its stop policy, and this tool \
            waits at most 45 seconds. Returns {name, stopped (it was running and has ended), \
            still_stopping (it had not ended when the wait ended; it still ends, so follow it \
            with process_list)}. An unknown name is a result flagged isError that lists the \
            valid names."
    )]
    async fn process_remove(&self, Parameters(arg): Parameters<NameArg>) -> ToolResult {
        let removed = self
            .blocking(move |deps| {
                let typed = process_name(arg.name())?;
                let stack = deps.stacks().stack(deps.root())?.ok_or_else(|| {
                    ConfigError::UnknownProject {
                        root: deps.root().as_ref().clone(),
                    }
                })?;
                let name = stack.require(&typed)?.name().clone();
                let project = stack.project();
                let client = ServiceClient::connect()?;
                let before = process::status(&client, project)?;
                let sent = SystemTime::now();
                deps.editor().remove(deps.root(), &name)?;
                let names = [name];
                let report = process::settle_removed(
                    &client,
                    project,
                    &before,
                    &names,
                    sent,
                    SETTLE_BOUND,
                    &mut Unwatched,
                )?;
                report
                    .processes()
                    .first()
                    .map(RemovedView::from)
                    .ok_or(ToolError::Client(ClientError::UnexpectedAnswer(
                        REMOVAL_UNREPORTED,
                    )))
            })
            .await?;
        structured(&removed)
    }

    /// Reads one setting, or all of them.
    #[tool(
        description = "Read the machine's muster settings. Takes key (optional). With key, \
            returns {key, value}; without, {settings: [{key, value}]} listing every setting. \
            Values are written as config_set reads them. An unknown key is a result flagged \
            isError that lists the valid keys."
    )]
    async fn config_get(&self, Parameters(arg): Parameters<ConfigGetArg>) -> ToolResult {
        let key = arg.key().as_deref().map(setting_key).transpose()?;
        let settings = self.deps.settings().load()?;
        match key {
            Some(key) => structured(&SettingView::of(&settings, key)),
            None => structured(&SettingsView::from(&settings)),
        }
    }

    /// Changes one setting.
    #[tool(
        description = "Change one of the machine's muster settings. Takes key and value, the \
            value written as config_get shows it (true, 24x80, 10MiB, on-failure, 5s). The \
            default-* settings apply to processes added afterwards; default-size, scrollback, \
            log-max-size and color-scheme to processes started afterwards. Returns {key, value}. \
            An unknown key or an invalid value is a result flagged isError that names the \
            accepted form."
    )]
    async fn config_set(&self, Parameters(arg): Parameters<ConfigSetArg>) -> ToolResult {
        let key = setting_key(arg.key())?;
        let change = SettingChange::parse(key, arg.value())?;
        let settings = self
            .blocking(move |deps| Ok(deps.editor().configure(&change)?))
            .await?;
        structured(&SettingView::of(&settings, key))
    }
}

impl CoordinationMcp {
    /// Runs `work` over this server's deps on a blocking thread, since it may
    /// write the database, reach the service or wait for a process.
    ///
    /// # Errors
    /// Returns the caller's mistake, such as an unknown name, or a fault of
    /// muster's.
    async fn blocking<T, Work>(&self, work: Work) -> Result<T, ToolFailure>
    where
        T: Send + 'static,
        Work: FnOnce(&McpDeps) -> Result<T, ToolError> + Send + 'static,
    {
        let deps = self.deps.clone();
        task::spawn_blocking(move || work(&deps))
            .await
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?
            .map_err(ToolFailure::from)
    }

    /// Runs `query` against this project through the service, starting or
    /// replacing the service as the CLI does. A folder that is not yet a
    /// project is refused before the service is reached.
    ///
    /// # Errors
    /// Returns the caller's mistake, such as an unknown name, or the fault of
    /// a service that cannot be reached.
    async fn ask<T, Query>(&self, query: Query) -> Result<T, ToolFailure>
    where
        T: Send + 'static,
        Query: FnOnce(&ServiceClient, &Project) -> Result<T, ToolError> + Send + 'static,
    {
        self.blocking(move |deps| {
            let project = deps.project()?;
            let client = ServiceClient::connect()?;
            query(&client, &project)
        })
        .await
    }

    /// Applies `action` to the process an agent named `name` and reports what
    /// became of it once it settled, or once [`SETTLE_BOUND`] passed.
    ///
    /// # Errors
    /// Returns the caller's mistake for a blank or unknown name, or the fault
    /// of a service that cannot be reached.
    async fn control(&self, name: &str, action: ProcessAction) -> ToolResult {
        let name = reference(name)?;
        let report = self
            .ask(move |client, project| {
                process::control_within(
                    client,
                    project,
                    process::named(project, &name),
                    action,
                    SETTLE_BOUND,
                    &mut Unwatched,
                )
            })
            .await?;
        action_result(&report)
    }
}

/// The scratchpad key an agent typed as `key`.
///
/// # Errors
/// Returns the caller's mistake for a blank key.
fn scratchpad_key(key: &str) -> Result<ScratchpadKey, ToolFailure> {
    ScratchpadKey::try_new(key.to_string())
        .map_err(|_| ToolError::InvalidNoteKey(key.to_string()).into())
}

/// The todo id an agent typed as `id`.
///
/// # Errors
/// Returns the caller's mistake for a blank id.
fn todo_id(id: &str) -> Result<TodoId, ToolFailure> {
    TodoId::try_new(id.to_string()).map_err(|_| ToolError::InvalidTodoId(id.to_string()).into())
}

/// The key-value key an agent typed as `key`.
///
/// # Errors
/// Returns the caller's mistake for a blank key.
fn kv_key(key: &str) -> Result<KvKey, ToolFailure> {
    KvKey::try_new(key.to_string()).map_err(|_| ToolError::InvalidKvKey(key.to_string()).into())
}

#[tool_handler]
impl ServerHandler for CoordinationMcp {
    fn get_info(&self) -> ServerInfo {
        let mut implementation = Implementation::default();
        implementation.name = MCP_SERVER_NAME.to_string();
        implementation.version = MCP_SERVER_VERSION.to_string();
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = implementation;
        info.instructions = Some(MCP_INSTRUCTIONS.to_string());
        info
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    use strum::IntoEnumIterator;
    use tempfile::TempDir;

    use super::*;
    use crate::{
        adapter::{
            client::{Action, Outcome, ProcessOutcome, ProjectIdentityView, WaitEnd},
            store::SqliteStore,
        },
        domain::{
            coordination::{Author, Todo},
            port::{ProjectStore, StackStore, StateChangeNotifier},
            value::{ProcessName, ProjectName},
        },
    };

    /// The database file each test's store opens.
    const DATABASE: &str = "muster.db";
    /// The project folder each test serves, inside its temporary directory.
    const PROJECT_DIR: &str = "shop";

    /// A [`StateChangeNotifier`] that records what it was told.
    #[derive(Default)]
    struct RecordingNotifier {
        stacks: Mutex<Vec<ProjectRoot>>,
        settings: Mutex<usize>,
    }

    impl StateChangeNotifier for RecordingNotifier {
        fn stack_changed(&self, root: &ProjectRoot) {
            self.stacks.lock().unwrap().push(root.clone());
        }

        fn settings_changed(&self) {
            *self.settings.lock().unwrap() += 1;
        }
    }

    /// A server over a fresh database, serving a folder no project claims
    /// yet, with the store and notifier it writes through.
    struct Fixture {
        _dir: TempDir,
        root: ProjectRoot,
        store: SqliteStore,
        notifier: Arc<RecordingNotifier>,
        server: CoordinationMcp,
    }

    /// A [`Fixture`] whose folder is not yet a project.
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join(PROJECT_DIR);
        fs::create_dir(&folder).unwrap();
        fs::create_dir(folder.join("frontend")).unwrap();
        let root = ProjectRoot::try_new(fs::canonicalize(&folder).unwrap()).unwrap();
        let store = SqliteStore::open(&dir.path().join(DATABASE)).unwrap();
        let notifier = Arc::new(RecordingNotifier::default());
        let deps = McpDeps::builder()
            .root(root.clone())
            .author(Author::agent("test"))
            .projects(Arc::new(store.clone()))
            .coordination(Arc::new(store.clone()))
            .stacks(Arc::new(store.clone()))
            .settings(Arc::new(store.clone()))
            .notifier(notifier.clone())
            .build();
        Fixture {
            _dir: dir,
            root,
            store,
            notifier,
            server: CoordinationMcp::new(deps),
        }
    }

    /// Builds a single-threaded runtime for the async tool calls.
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
    }

    /// The text of a tool result's first content block.
    fn result_text(result: &CallToolResult) -> String {
        match &result.content[0] {
            ContentBlock::Text(text) => text.text.clone(),
            other => panic!("expected text content, got {other:?}"),
        }
    }

    /// What the agent receives for `result`.
    fn answered(result: ToolResult) -> Result<CallToolResult, ErrorData> {
        result.or_else(ToolFailure::answer)
    }

    /// The text of the error result an agent receives for `result`.
    fn refusal(result: ToolResult) -> String {
        let answer = answered(result).expect("a result, not a protocol error");
        assert_eq!(answer.is_error, Some(true));
        result_text(&answer)
    }

    /// The scratchpad `key` with `body`, to set.
    fn note(key: &str, body: &str) -> Parameters<ScratchpadSetArg> {
        Parameters(
            ScratchpadSetArg::builder()
                .key(key.to_string())
                .body(body.to_string())
                .build(),
        )
    }

    /// `process_add` arguments for `name` running `command`.
    fn add(name: &str, command: &str) -> ProcessAddArg {
        ProcessAddArg::builder()
            .name(name.to_string())
            .command(command.to_string())
            .build()
    }

    /// `process_set` arguments read from the JSON an agent sends.
    fn set(json: serde_json::Value) -> Parameters<ProcessSetArg> {
        Parameters(serde_json::from_value(json).unwrap())
    }

    /// The process `name` as a tool argument.
    fn named(name: &str) -> Parameters<NameArg> {
        Parameters(NameArg::builder().name(name.to_string()).build())
    }

    /// Registers the fixture's folder by adding a `web` process to it.
    fn register(fixture: &Fixture) {
        runtime().block_on(async {
            fixture
                .server
                .process_add(Parameters(add("web", "npm run dev")))
                .await
                .unwrap();
        });
    }

    /// A start report for `api` whose outcome is `outcome` and whose wait
    /// ended as `wait_ended`.
    fn report(outcome: Outcome, wait_ended: WaitEnd) -> ActionReport {
        ActionReport::builder()
            .project(
                ProjectIdentityView::builder()
                    .name(ProjectName::try_new("repo").unwrap())
                    .root(PathBuf::from("/repo"))
                    .build(),
            )
            .action(Action::Start)
            .processes(vec![
                ProcessOutcome::builder()
                    .name(ProcessName::try_new("api").unwrap())
                    .outcome(outcome)
                    .status(None)
                    .build(),
            ])
            .notices(Vec::new())
            .other_notices(Vec::new())
            .wait_ended(wait_ended)
            .build()
    }

    /// A `scratchpad_set` tool call writes through to the store, and
    /// `scratchpad_list` reads it back as structured data - the store-backed
    /// round trip an agent depends on.
    #[test]
    fn a_set_tool_call_writes_through_to_the_store() {
        let fixture = fixture();
        register(&fixture);
        let server = &fixture.server;

        runtime().block_on(async {
            let saved = server
                .scratchpad_set(note("plan", "ship it"))
                .await
                .unwrap();
            assert_eq!(result_text(&saved), "saved 'plan'");

            let listed = server
                .scratchpad_list(Parameters(ListArg::builder().build()))
                .await
                .unwrap();
            let index = listed
                .structured_content
                .expect("list returns structured data");
            let notes = &index[SCRATCHPADS];
            assert_eq!(notes[0]["key"], "plan");
            assert_eq!(notes[0]["author"], "test");
            assert!(
                notes[0].get("updated_at").is_some(),
                "updated_at is surfaced"
            );
            assert!(
                notes[0].get("body").is_none(),
                "the index omits bodies so a long note cannot flood the caller"
            );

            let fetched = server
                .scratchpad_get(Parameters(
                    KeyArg::builder().key("plan".to_string()).build(),
                ))
                .await
                .unwrap();
            assert_eq!(
                fetched.structured_content.expect("structured note")["body"],
                "ship it"
            );
        });
    }

    /// Coordination in a folder no project claims is the caller's mistake,
    /// answered with the tool that registers the folder.
    #[test]
    fn coordination_outside_a_project_names_process_add() {
        let fixture = fixture();

        runtime().block_on(async {
            let text = refusal(fixture.server.scratchpad_set(note("plan", "x")).await);
            assert!(text.contains("process_add"), "{text}");
            assert!(!text.contains("`muster"), "{text}");
        });
    }

    /// A blank key is the caller's mistake, answered as an error result
    /// before the store is touched.
    #[test]
    fn a_blank_key_is_an_error_result() {
        let fixture = fixture();

        runtime().block_on(async {
            let result = answered(fixture.server.scratchpad_set(note("   ", "x")).await).unwrap();
            assert_eq!(result.is_error, Some(true));
            assert_eq!(
                result.structured_content.expect("structured")["error"]["kind"],
                "invalid-input"
            );
        });
    }

    /// A blank process name is the caller's mistake, answered before the
    /// service is asked.
    #[test]
    fn a_blank_process_name_is_an_error_result() {
        let fixture = fixture();
        runtime().block_on(async {
            refusal(fixture.server.process_output(named("   ")).await);
        });
    }

    /// `process_add` in a folder no project claims registers it, stores the
    /// definition with the default settings for what it left out, and tells
    /// a running service which project changed.
    #[test]
    fn process_add_registers_the_folder() {
        let fixture = fixture();
        assert!(fixture.store.project_at(&fixture.root).unwrap().is_none());

        runtime().block_on(async {
            let added = fixture
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
                .await
                .unwrap();
            let definition = added.structured_content.expect("the stored definition");
            assert_eq!(definition["name"], "web");
            assert_eq!(definition["command"], "npm run dev");
            assert_eq!(definition["working_dir"], "frontend");
            assert_eq!(definition["environment"]["PORT"], "5173");
            assert_eq!(definition["restart"], "on-failure");
            assert_eq!(definition["autostart"], false, "the seeded default");
            assert_eq!(definition["stop"]["signal"], "terminate");
        });

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
            assert!(invalid.contains("restart"), "{invalid}");
            assert!(invalid.contains("on-failure"), "{invalid}");
        });

        let stack = fixture
            .store
            .stack(&fixture.root)
            .unwrap()
            .expect("a stack");
        assert_eq!(stack.names().len(), 1);
        assert_eq!(fixture.notifier.stacks.lock().unwrap().len(), 1);
    }

    /// `process_set` changes only the fields it is given and reports the
    /// process as not running when no service has it loaded.
    #[test]
    fn process_set_changes_the_given_fields() {
        let fixture = fixture();
        register(&fixture);

        runtime().block_on(async {
            let changed = fixture
                .server
                .process_set(set(serde_json::json!({
                    "name": "web",
                    "restart": "always",
                    "environment_set": { "PORT": "3000" },
                    "working_dir": "frontend",
                })))
                .await
                .unwrap();
            let content = changed.structured_content.expect("structured");
            assert_eq!(content[PROCESS]["restart"], "always");
            assert_eq!(content[PROCESS]["environment"]["PORT"], "3000");
            assert_eq!(content[PROCESS]["working_dir"], "frontend");
            assert_eq!(content[PROCESS]["command"], "npm run dev");
            assert_eq!(content[RUNNING], false);

            let cleared = fixture
                .server
                .process_set(set(
                    serde_json::json!({ "name": "web", "working_dir": null }),
                ))
                .await
                .unwrap();
            assert!(
                cleared.structured_content.expect("structured")[PROCESS]["working_dir"].is_null()
            );
        });

        assert_eq!(fixture.notifier.stacks.lock().unwrap().len(), 3);
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

        assert_eq!(fixture.notifier.stacks.lock().unwrap().len(), 1);
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

    /// `config_get` reads every setting or one; an unknown key lists the
    /// valid ones.
    #[test]
    fn config_get_reads_one_or_every_setting() {
        let fixture = fixture();

        runtime().block_on(async {
            let all = fixture
                .server
                .config_get(Parameters(ConfigGetArg::builder().build()))
                .await
                .unwrap()
                .structured_content
                .expect("structured");
            let listed = all["settings"].as_array().expect("a list of settings");
            assert_eq!(listed.len(), SettingKey::iter().count());
            for key in SettingKey::iter() {
                assert!(
                    listed.iter().any(|entry| entry["key"] == key.to_string()),
                    "{key} is listed"
                );
            }

            let one = fixture
                .server
                .config_get(Parameters(
                    ConfigGetArg::builder()
                        .key(Some("default-restart".to_string()))
                        .build(),
                ))
                .await
                .unwrap()
                .structured_content
                .expect("structured");
            assert_eq!(
                one,
                serde_json::json!({ "key": "default-restart", "value": "never" })
            );

            let unknown = refusal(
                fixture
                    .server
                    .config_get(Parameters(
                        ConfigGetArg::builder()
                            .key(Some("restart".to_string()))
                            .build(),
                    ))
                    .await,
            );
            assert!(unknown.contains("default-restart"), "{unknown}");
        });
    }

    /// `config_set` stores a value `config_get` reads back and tells a
    /// running service; an invalid value names the accepted form and stores
    /// nothing.
    #[test]
    fn config_set_stores_and_announces_a_setting() {
        let fixture = fixture();
        let server = &fixture.server;

        runtime().block_on(async {
            let stored = server
                .config_set(Parameters(
                    ConfigSetArg::builder()
                        .key("default-restart".to_string())
                        .value("on-failure".to_string())
                        .build(),
                ))
                .await
                .unwrap();
            assert_eq!(
                stored.structured_content.expect("structured"),
                serde_json::json!({ "key": "default-restart", "value": "on-failure" })
            );

            let invalid = refusal(
                server
                    .config_set(Parameters(
                        ConfigSetArg::builder()
                            .key("default-stop-grace".to_string())
                            .value("0s".to_string())
                            .build(),
                    ))
                    .await,
            );
            assert!(invalid.contains("default-stop-grace"), "{invalid}");

            let added = server
                .process_add(Parameters(add("web", "npm run dev")))
                .await
                .unwrap();
            assert_eq!(
                added.structured_content.expect("structured")["restart"],
                "on-failure",
                "a new process takes the changed default"
            );
        });

        assert_eq!(*fixture.notifier.settings.lock().unwrap(), 1);
    }

    /// The connection snippet is valid JSON that launches this binary for the
    /// project's folder under the muster server key.
    #[test]
    fn connection_config_is_valid_and_points_at_this_binary() {
        let root = ProjectRoot::try_new(PathBuf::from("/repo")).unwrap();
        let json = connection_config(Path::new("/usr/local/bin/muster"), &root);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let entry = &parsed["mcpServers"][MCP_CONNECTION_KEY];
        assert_eq!(entry["command"], "/usr/local/bin/muster");
        assert_eq!(
            entry["args"],
            serde_json::json!(["mcp", "--project", "/repo"])
        );
    }

    /// A page size defaults, clamps to the cap, and never yields an empty page.
    #[test]
    fn page_size_defaults_and_clamps() {
        assert_eq!(page_size(None), DEFAULT_LIST_LIMIT);
        assert_eq!(page_size(Some(5)), 5);
        assert_eq!(page_size(Some(0)), 1, "a zero page would return nothing");
        assert_eq!(page_size(Some(10_000)), MAX_LIST_LIMIT);
    }

    /// `scratchpad_list` returns at most the requested page, so a project with
    /// many notes cannot flood the caller in one call.
    #[test]
    fn scratchpad_list_honours_the_limit() {
        let fixture = fixture();
        register(&fixture);
        let server = &fixture.server;
        runtime().block_on(async {
            for key in ["a", "b", "c"] {
                server.scratchpad_set(note(key, "x")).await.unwrap();
            }
            let listed = server
                .scratchpad_list(Parameters(ListArg::builder().limit(Some(2)).build()))
                .await
                .unwrap();
            let index = listed.structured_content.expect("structured index");
            assert_eq!(index[SCRATCHPADS].as_array().expect("an array").len(), 2);
        });
    }

    /// A lifecycle report is structured data, flagged as an error when the
    /// process did not end up as asked so an agent cannot read it as done.
    #[test]
    fn a_failed_action_report_is_an_error_result() {
        let started = action_result(&report(Outcome::Started, WaitEnd::Settled)).unwrap();
        assert_eq!(started.is_error, Some(false));
        let content = started.structured_content.expect("a structured report");
        assert_eq!(content["action"], "start");
        assert_eq!(content["processes"][0]["outcome"], "started");

        let crashed = action_result(&report(Outcome::Crashed, WaitEnd::Settled)).unwrap();
        assert_eq!(crashed.is_error, Some(true));
        assert_eq!(
            crashed.structured_content.expect("a structured report")["processes"][0]["outcome"],
            "crashed"
        );
    }

    /// A process still stopping when this server's bound cut the wait short
    /// is on its way, not failed; one still stopping when the action's own
    /// timeout expired failed to settle.
    #[test]
    fn a_wait_cut_short_is_no_error_and_a_timed_out_one_is() {
        let cut = action_result(&report(Outcome::StillStopping, WaitEnd::CutShort)).unwrap();
        assert_eq!(cut.is_error, Some(false));
        let content = cut.structured_content.expect("a structured report");
        assert_eq!(content["wait_ended"], "cut-short");
        assert_eq!(content["processes"][0]["outcome"], "still-stopping");

        let timed_out = action_result(&report(Outcome::StillStopping, WaitEnd::TimedOut)).unwrap();
        assert_eq!(timed_out.is_error, Some(true));
        assert_eq!(
            timed_out.structured_content.expect("a structured report")["wait_ended"],
            "timed-out"
        );
        assert_eq!(
            action_result(&report(Outcome::Crashed, WaitEnd::CutShort))
                .unwrap()
                .is_error,
            Some(true),
            "a settled failure stays a failure whatever cut the wait"
        );
    }

    /// A list is held under its key, since a structured result is always an
    /// object.
    #[test]
    fn a_listing_is_an_object() {
        let listed = listing(TODOS, &Vec::<Todo>::new()).unwrap();
        assert_eq!(
            listed.structured_content,
            Some(serde_json::json!({ "todos": [] }))
        );
    }

    /// The advertised server info names the server and enables the tools
    /// capability.
    #[test]
    fn server_info_advertises_tools() {
        let fixture = fixture();
        let info = fixture.server.get_info();
        assert_eq!(info.server_info.name, MCP_SERVER_NAME);
        assert!(info.capabilities.tools.is_some());
    }
}
