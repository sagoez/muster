use std::path::{Path, PathBuf};

use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::stdio,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    adapter::bridge::{
        ControlAction, ControlOutcome, SendOutcome, WorkspaceRequest, WorkspaceResponse,
    },
    domain::{
        config::ConfigError,
        coordination::{Author, KvKey, ScratchpadKey, TodoId, TodoStatus, TodoTitle},
        port::CoordinationStore,
    },
};

/// Server name reported to connecting agents.
const MCP_SERVER_NAME: &str = "muster-coordination";
/// Key muster's server is registered under in an agent's MCP config; it
/// namespaces the tools an agent sees (e.g. `muster` -> `scratchpad_set`).
const MCP_CONNECTION_KEY: &str = "muster";
/// Subcommand an agent invokes to start the server.
const MCP_SUBCOMMAND: &str = "mcp";
/// Config-path flag passed to the server subcommand.
const MCP_CONFIG_FLAG: &str = "--config";
/// Server version reported to connecting agents.
const MCP_SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Guidance shown to agents that connect to the coordination server.
const MCP_INSTRUCTIONS: &str = "Shared coordination state for this muster project: durable \
    scratchpad notes, structured todos, and key-values that every agent and the human read and \
    write. Use scratchpads to pass context and feedback, todos to track work, and key-values for \
    small shared facts (a chosen port, a build id). Live tools (process_list) read \
    the running workspace when muster is open in the project.";
/// Entries a list tool returns when the caller does not ask for a size.
const DEFAULT_LIST_LIMIT: usize = 50;
/// Hard cap on a list tool's page, so one call cannot flood the caller's context.
const MAX_LIST_LIMIT: usize = 200;
/// Reply when a live query finds no workspace running for the project.
const NO_WORKSPACE: &str = "no running muster workspace for this project; start it by running `muster` in the project \
     directory";

/// A failure starting or running the coordination MCP server over stdio.
#[derive(Debug, Error)]
pub enum McpError {
    /// The server could not be initialized on the transport.
    #[error("could not start the MCP server: {0}")]
    Serve(String),
    /// The server stopped with an error rather than a clean shutdown.
    #[error("the MCP server stopped abnormally: {0}")]
    Wait(String),
}

/// Builds the pretty-printed JSON snippet an agent pastes into its MCP-servers
/// config to launch muster's coordination server for `project`. `executable` is
/// the muster binary path, so the entry works even when muster is not on `PATH`.
pub fn connection_config(executable: &Path, project: &Path) -> String {
    let value = serde_json::json!({
        "mcpServers": {
            MCP_CONNECTION_KEY: {
                "command": executable.display().to_string(),
                "args": [
                    MCP_SUBCOMMAND,
                    MCP_CONFIG_FLAG,
                    project.display().to_string(),
                ],
            },
        },
    });
    serde_json::to_string_pretty(&value).unwrap_or_default()
}

/// Serves the coordination store for `project` as an MCP server over stdio,
/// blocking until the client disconnects. Intended to run as a child process an
/// agent spawns, so the OS process boundary is the trust boundary; no port and no
/// token are exposed.
///
/// # Errors
/// Returns [`McpError`] if the server cannot be initialized or stops abnormally.
pub async fn serve_stdio(
    store: Box<dyn CoordinationStore + Send + Sync>,
    project: PathBuf,
    author: Author,
) -> Result<(), McpError> {
    let service = CoordinationMcp::new(store, project, author)
        .serve(stdio())
        .await
        .map_err(|error| McpError::Serve(error.to_string()))?;
    service
        .waiting()
        .await
        .map_err(|error| McpError::Wait(error.to_string()))?;
    Ok(())
}

/// Key of a scratchpad or key-value entry.
#[derive(Debug, Deserialize, JsonSchema)]
struct KeyArg {
    /// The entry's key.
    key: String,
}

/// A scratchpad key and its markdown body.
#[derive(Debug, Deserialize, JsonSchema)]
struct ScratchpadSetArg {
    /// The note's key.
    key: String,
    /// The note's markdown body.
    body: String,
}

/// A new todo's title and its dependency ids.
#[derive(Debug, Deserialize, JsonSchema)]
struct TodoAddArg {
    /// What the task is.
    title: String,
    /// Ids of todos this one depends on.
    #[serde(default)]
    deps: Vec<String>,
}

/// A todo id alone.
#[derive(Debug, Deserialize, JsonSchema)]
struct TodoIdArg {
    /// The todo's id.
    id: String,
}

/// A todo id and its new status.
#[derive(Debug, Deserialize, JsonSchema)]
struct TodoStatusArg {
    /// The todo's id.
    id: String,
    /// New status: `pending`, `in-progress`, or `done`.
    status: String,
}

/// A key-value key and its value.
#[derive(Debug, Deserialize, JsonSchema)]
struct KvSetArg {
    /// The entry's key.
    key: String,
    /// The value to store.
    value: String,
}

/// The id of a process/pane to read, as reported by `process_list`.
#[derive(Debug, Deserialize, JsonSchema)]
struct PaneArg {
    /// Process id from `process_list`.
    id: u64,
}

/// Text to type into a process's terminal.
#[derive(Debug, Deserialize, JsonSchema)]
struct SendInputArg {
    /// Process id from `process_list`.
    id: u64,
    /// Text to write to the process's terminal.
    text: String,
    /// Append a carriage return to submit the line (run a command, answer a
    /// prompt). Defaults to false, so the text is typed without submitting.
    #[serde(default)]
    enter: bool,
}

/// How many entries a list tool should return.
#[derive(Debug, Deserialize, JsonSchema)]
struct ListArg {
    /// Maximum entries to return. Defaults to 50, capped at 200.
    #[serde(default)]
    limit: Option<usize>,
}

/// A scratchpad without its body: the index `scratchpad_list` returns, so a
/// project full of long notes can never flood the caller in one call. The body is
/// fetched per key with `scratchpad_get`.
#[derive(Debug, Serialize)]
struct ScratchpadEntry {
    key: String,
    author: Author,
    updated_at: u64,
}

/// Resolves a requested page size to the allowed range.
fn page_size(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_LIST_LIMIT).clamp(1, MAX_LIST_LIMIT)
}

/// MCP driving adapter exposing one project's [`CoordinationStore`] as tools.
/// Writes are attributed to `author` - the agent this server was launched for -
/// so every coordination entry an agent creates carries its identity.
pub struct CoordinationMcp {
    store: Box<dyn CoordinationStore + Send + Sync>,
    project: PathBuf,
    author: Author,
}

/// Wraps a store failure as an MCP internal error.
fn store_error(error: ConfigError) -> ErrorData {
    ErrorData::internal_error(error.to_string(), None)
}

/// Wraps an invalid-argument condition as an MCP invalid-params error.
fn bad_arg(message: impl Into<std::borrow::Cow<'static, str>>) -> ErrorData {
    ErrorData::invalid_params(message, None)
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
/// happen for the coordination entities.
fn structured<T: Serialize>(value: &T) -> Result<CallToolResult, ErrorData> {
    serde_json::to_value(value)
        .map(CallToolResult::structured)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))
}

#[tool_router]
impl CoordinationMcp {
    /// Builds the server over `store`, scoped to `project`, writing as `author`.
    pub fn new(
        store: Box<dyn CoordinationStore + Send + Sync>,
        project: PathBuf,
        author: Author,
    ) -> Self {
        Self {
            store,
            project,
            author,
        }
    }

    /// Lists every scratchpad in this project as structured objects.
    #[tool(
        description = "List every shared scratchpad note in this project (other agents and the \
            human read and write these). Returns an INDEX - an array of {key, author, updated_at} \
            without bodies, newest page first - so a project of long notes cannot flood you. Read \
            a body with scratchpad_get. `limit` defaults to 50, capped at 200."
    )]
    async fn scratchpad_list(
        &self,
        Parameters(ListArg { limit }): Parameters<ListArg>,
    ) -> Result<CallToolResult, ErrorData> {
        let notes = self.store.scratchpads(&self.project).map_err(store_error)?;
        let page: Vec<ScratchpadEntry> = notes
            .iter()
            .take(page_size(limit))
            .map(|note| ScratchpadEntry {
                key: note.key().as_ref().to_string(),
                author: note.author().clone(),
                updated_at: *note.updated_at(),
            })
            .collect();
        structured(&page)
    }

    /// Reads one scratchpad.
    #[tool(
        description = "Read one shared scratchpad note by key. Returns {key, body, updated_at}; \
            errors if no note has that key."
    )]
    async fn scratchpad_get(
        &self,
        Parameters(KeyArg { key }): Parameters<KeyArg>,
    ) -> Result<CallToolResult, ErrorData> {
        let key = ScratchpadKey::try_new(key).map_err(|error| bad_arg(error.to_string()))?;
        match self
            .store
            .scratchpad(&self.project, &key)
            .map_err(store_error)?
        {
            Some(note) => structured(&note),
            None => Err(bad_arg(format!("no scratchpad '{}'", key.as_ref()))),
        }
    }

    /// Creates or replaces a scratchpad.
    #[tool(
        description = "Create or replace a shared scratchpad note (markdown body). Visible to \
            other agents and the human. Use it to pass context, plans, or feedback."
    )]
    async fn scratchpad_set(
        &self,
        Parameters(ScratchpadSetArg { key, body }): Parameters<ScratchpadSetArg>,
    ) -> Result<CallToolResult, ErrorData> {
        let key = ScratchpadKey::try_new(key).map_err(|error| bad_arg(error.to_string()))?;
        self.store
            .set_scratchpad(&self.project, &self.author, &key, &body)
            .map_err(store_error)?;
        Ok(text(format!("saved '{}'", key.as_ref())))
    }

    /// Deletes a scratchpad.
    #[tool(description = "Delete a scratchpad note by key.")]
    async fn scratchpad_delete(
        &self,
        Parameters(KeyArg { key }): Parameters<KeyArg>,
    ) -> Result<CallToolResult, ErrorData> {
        let key = ScratchpadKey::try_new(key).map_err(|error| bad_arg(error.to_string()))?;
        let removed = self
            .store
            .delete_scratchpad(&self.project, &key)
            .map_err(store_error)?;
        Ok(text(if removed {
            format!("deleted '{}'", key.as_ref())
        } else {
            format!("no scratchpad '{}'", key.as_ref())
        }))
    }

    /// Lists every todo as structured objects, including dependencies.
    #[tool(
        description = "List every shared todo in this project. Returns an array of {id, title, \
            status (pending|in-progress|done), deps (ids this todo depends on), author, \
            updated_at}. `limit` defaults to 50, capped at 200."
    )]
    async fn todo_list(
        &self,
        Parameters(ListArg { limit }): Parameters<ListArg>,
    ) -> Result<CallToolResult, ErrorData> {
        let todos = self.store.todos(&self.project).map_err(store_error)?;
        let page: Vec<_> = todos.into_iter().take(page_size(limit)).collect();
        structured(&page)
    }

    /// Reads one todo, including its dependencies.
    #[tool(
        description = "Read one todo by id. Returns {id, title, status, deps, updated_at}; errors \
            if no todo has that id."
    )]
    async fn todo_get(
        &self,
        Parameters(TodoIdArg { id }): Parameters<TodoIdArg>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = TodoId::try_new(id).map_err(|error| bad_arg(error.to_string()))?;
        match self.store.todo(&self.project, &id).map_err(store_error)? {
            Some(todo) => structured(&todo),
            None => Err(bad_arg(format!("no todo '{}'", id.as_ref()))),
        }
    }

    /// Adds a new pending todo, returning the created todo.
    #[tool(
        description = "Add a new shared, pending todo. `deps` are ids of todos this one depends \
            on (optional). Returns the created todo {id, title, status, deps, updated_at}."
    )]
    async fn todo_add(
        &self,
        Parameters(TodoAddArg { title, deps }): Parameters<TodoAddArg>,
    ) -> Result<CallToolResult, ErrorData> {
        let title = TodoTitle::try_new(title).map_err(|error| bad_arg(error.to_string()))?;
        let deps = deps
            .into_iter()
            .map(|dep| TodoId::try_new(dep).map_err(|error| bad_arg(error.to_string())))
            .collect::<Result<Vec<_>, _>>()?;
        let created = self
            .store
            .add_todo(&self.project, &self.author, &title, &deps)
            .map_err(store_error)?;
        structured(&created)
    }

    /// Sets a todo's lifecycle status.
    #[tool(description = "Set a todo's status: pending, in-progress, or done.")]
    async fn todo_set_status(
        &self,
        Parameters(TodoStatusArg { id, status }): Parameters<TodoStatusArg>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = TodoId::try_new(id).map_err(|error| bad_arg(error.to_string()))?;
        let status = status
            .parse::<TodoStatus>()
            .map_err(|_| bad_arg(format!("'{status}' is not a valid status")))?;
        if self
            .store
            .set_todo_status(&self.project, &self.author, &id, status)
            .map_err(store_error)?
        {
            Ok(text(format!("{id} is {status}")))
        } else {
            Err(bad_arg(format!("no todo '{id}'")))
        }
    }

    /// Deletes a todo.
    #[tool(description = "Delete a todo by id.")]
    async fn todo_delete(
        &self,
        Parameters(TodoIdArg { id }): Parameters<TodoIdArg>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = TodoId::try_new(id).map_err(|error| bad_arg(error.to_string()))?;
        let removed = self
            .store
            .delete_todo(&self.project, &id)
            .map_err(store_error)?;
        Ok(text(if removed {
            format!("deleted '{id}'")
        } else {
            format!("no todo '{id}'")
        }))
    }

    /// Lists every key-value entry as structured objects.
    #[tool(
        description = "List every shared key-value entry in this project. Returns an array of \
            {key, value, author, updated_at}. `limit` defaults to 50, capped at 200."
    )]
    async fn kv_list(
        &self,
        Parameters(ListArg { limit }): Parameters<ListArg>,
    ) -> Result<CallToolResult, ErrorData> {
        let entries = self.store.values(&self.project).map_err(store_error)?;
        let page: Vec<_> = entries.into_iter().take(page_size(limit)).collect();
        structured(&page)
    }

    /// Reads one key-value entry.
    #[tool(
        description = "Read one shared key-value entry by key. Returns {key, value, updated_at}; \
            errors if no entry has that key."
    )]
    async fn kv_get(
        &self,
        Parameters(KeyArg { key }): Parameters<KeyArg>,
    ) -> Result<CallToolResult, ErrorData> {
        let key = KvKey::try_new(key).map_err(|error| bad_arg(error.to_string()))?;
        match self.store.value(&self.project, &key).map_err(store_error)? {
            Some(entry) => structured(&entry),
            None => Err(bad_arg(format!("no value '{}'", key.as_ref()))),
        }
    }

    /// Creates or replaces a key-value entry.
    #[tool(
        description = "Create or replace a shared key-value entry (small facts other agents need: \
            a chosen port, a build id, a flag)."
    )]
    async fn kv_set(
        &self,
        Parameters(KvSetArg { key, value }): Parameters<KvSetArg>,
    ) -> Result<CallToolResult, ErrorData> {
        let key = KvKey::try_new(key).map_err(|error| bad_arg(error.to_string()))?;
        self.store
            .set_value(&self.project, &self.author, &key, &value)
            .map_err(store_error)?;
        Ok(text(format!("saved '{}'", key.as_ref())))
    }

    /// Deletes a key-value entry.
    #[tool(description = "Delete a key-value entry by key.")]
    async fn kv_delete(
        &self,
        Parameters(KeyArg { key }): Parameters<KeyArg>,
    ) -> Result<CallToolResult, ErrorData> {
        let key = KvKey::try_new(key).map_err(|error| bad_arg(error.to_string()))?;
        let removed = self
            .store
            .delete_value(&self.project, &key)
            .map_err(store_error)?;
        Ok(text(if removed {
            format!("deleted '{}'", key.as_ref())
        } else {
            format!("no value '{}'", key.as_ref())
        }))
    }

    /// Lists the processes running in this project's live workspace.
    #[tool(
        description = "List the processes running in this project's live muster workspace \
            (agents, terminals, commands) with their lifecycle state and activity. Reads the \
            running TUI over its local socket; returns a note if no workspace is running here."
    )]
    async fn process_list(&self) -> Result<CallToolResult, ErrorData> {
        match self.query_workspace(WorkspaceRequest::ListProcesses)? {
            Some(WorkspaceResponse::Processes(processes)) => structured(&processes),
            Some(WorkspaceResponse::Unavailable) => Err(Self::workspace_unavailable()),
            Some(_) => Err(Self::unexpected_response()),
            None => Ok(text(NO_WORKSPACE)),
        }
    }

    /// Reads the latest on-screen output of a process's terminal.
    #[tool(
        description = "Read the latest on-screen output of a process's terminal by its id (from \
            process_list) - what a dev server, test run, or agent last printed. Returns the text, \
            or a note if no workspace is running or no process has that id."
    )]
    async fn process_output(
        &self,
        Parameters(PaneArg { id }): Parameters<PaneArg>,
    ) -> Result<CallToolResult, ErrorData> {
        match self.query_workspace(WorkspaceRequest::PaneOutput { pane: id })? {
            Some(WorkspaceResponse::PaneOutput(Some(output))) => Ok(text(output)),
            Some(WorkspaceResponse::PaneOutput(None)) => {
                Err(bad_arg(format!("no process with id {id}")))
            },
            Some(WorkspaceResponse::Unavailable) => Err(Self::workspace_unavailable()),
            Some(_) => Err(Self::unexpected_response()),
            None => Ok(text(NO_WORKSPACE)),
        }
    }

    /// Lists this project's durable agent sessions.
    #[tool(
        description = "List this project's durable agent sessions: {name, tool, state \
            (pending|open|closed), resumable}. `resumable` means a native conversation was \
            captured, so the agent can be resumed rather than started fresh."
    )]
    async fn session_list(&self) -> Result<CallToolResult, ErrorData> {
        match self.query_workspace(WorkspaceRequest::ListSessions)? {
            Some(WorkspaceResponse::Sessions(sessions)) => structured(&sessions),
            Some(WorkspaceResponse::Unavailable) => Err(Self::workspace_unavailable()),
            Some(_) => Err(Self::unexpected_response()),
            None => Ok(text(NO_WORKSPACE)),
        }
    }

    /// Starts a process.
    #[tool(
        description = "Start a process by id (from process_list) if it is not already running. \
            The action is requested on the workspace; poll process_list to see the result."
    )]
    async fn process_start(
        &self,
        Parameters(PaneArg { id }): Parameters<PaneArg>,
    ) -> Result<CallToolResult, ErrorData> {
        self.control(id, ControlAction::Start, "start")
    }

    /// Stops a process.
    #[tool(
        description = "Stop a process by id (from process_list) without its restart policy \
            respawning it. The stop is graceful, so it may take effect after the configured \
            grace period; poll process_list to see the result."
    )]
    async fn process_stop(
        &self,
        Parameters(PaneArg { id }): Parameters<PaneArg>,
    ) -> Result<CallToolResult, ErrorData> {
        self.control(id, ControlAction::Stop, "stop")
    }

    /// Restarts a process.
    #[tool(
        description = "Restart a process by id (from process_list), regardless of its configured \
            restart policy. Poll process_list to see the result."
    )]
    async fn process_restart(
        &self,
        Parameters(PaneArg { id }): Parameters<PaneArg>,
    ) -> Result<CallToolResult, ErrorData> {
        self.control(id, ControlAction::Restart, "restart")
    }

    /// Types text into a process's terminal.
    #[tool(
        description = "Type text into a process's terminal by id (from process_list), as if the \
            human typed it - to answer a prompt, run a command in a shell, or unblock a waiting \
            process. Set enter=true to submit the line. Errors if the process is not running or \
            the id is unknown."
    )]
    async fn process_send(
        &self,
        Parameters(SendInputArg {
            id,
            text: input,
            enter,
        }): Parameters<SendInputArg>,
    ) -> Result<CallToolResult, ErrorData> {
        let data = if enter { format!("{input}\r") } else { input };
        match self.query_workspace(WorkspaceRequest::SendInput { pane: id, data })? {
            Some(WorkspaceResponse::Sent(SendOutcome::Delivered)) => Ok(text("sent")),
            Some(WorkspaceResponse::Sent(SendOutcome::NotRunning)) => {
                Err(bad_arg(format!("process {id} is not running")))
            },
            Some(WorkspaceResponse::Sent(SendOutcome::Unknown)) => {
                Err(bad_arg(format!("no process with id {id}")))
            },
            Some(WorkspaceResponse::Unavailable) => Err(Self::workspace_unavailable()),
            Some(_) => Err(Self::unexpected_response()),
            None => Ok(text(NO_WORKSPACE)),
        }
    }
}

impl CoordinationMcp {
    /// Sends a workspace query to the running TUI over this project's socket and
    /// returns the structured reply, or a note when no workspace is running here.
    ///
    /// # Errors
    /// Returns an internal error only if the state directory cannot be resolved;
    /// an unreachable socket is reported as a normal "no workspace" result.
    fn query_workspace(
        &self,
        request: WorkspaceRequest,
    ) -> Result<Option<WorkspaceResponse>, ErrorData> {
        #[cfg(unix)]
        {
            use crate::adapter::ipc;

            let Some(path) = ipc::socket_path(&self.project) else {
                return Err(ErrorData::internal_error(
                    "no state directory available",
                    None,
                ));
            };
            // An unreachable socket means no workspace is running here, which is a
            // normal answer (None), not a tool error.
            Ok(ipc::request::<WorkspaceRequest, WorkspaceResponse>(&path, &request).ok())
        }
        #[cfg(not(unix))]
        {
            let _ = request;
            Ok(None)
        }
    }

    /// Requests `action` on the process `id`, mapping the outcome to a tool
    /// result. `verb` names the action in the acknowledgement.
    ///
    /// # Errors
    /// Returns invalid-params for an unknown id, or an internal error if the
    /// workspace could not answer.
    fn control(
        &self,
        id: u64,
        action: ControlAction,
        verb: &str,
    ) -> Result<CallToolResult, ErrorData> {
        match self.query_workspace(WorkspaceRequest::ControlProcess { pane: id, action })? {
            Some(WorkspaceResponse::Controlled(ControlOutcome::Requested)) => {
                Ok(text(format!("requested {verb} for process {id}")))
            },
            Some(WorkspaceResponse::Controlled(ControlOutcome::Unknown)) => {
                Err(bad_arg(format!("no process with id {id}")))
            },
            Some(WorkspaceResponse::Unavailable) => Err(Self::workspace_unavailable()),
            Some(_) => Err(Self::unexpected_response()),
            None => Ok(text(NO_WORKSPACE)),
        }
    }

    /// Reports the unexpected-response case: the workspace answered a different
    /// query than was asked (a protocol drift), surfaced as an internal error.
    fn unexpected_response() -> ErrorData {
        ErrorData::internal_error("unexpected workspace response", None)
    }

    /// Reports that the workspace was reachable but could not answer (it is
    /// shutting down, or dropped the request). An error rather than an empty
    /// result, so a caller never reads "could not ask" as "nothing is running" -
    /// and, for a write, never assumes input was delivered.
    fn workspace_unavailable() -> ErrorData {
        ErrorData::internal_error(
            "the muster workspace could not answer (it may be shutting down)",
            None,
        )
    }
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
    use std::sync::Mutex;

    use super::*;
    use crate::domain::coordination::{Author, KeyValue, Scratchpad, Todo};

    /// An in-memory, thread-safe coordination store recording scratchpads.
    #[derive(Default)]
    struct FakeStore {
        notes: Mutex<Vec<Scratchpad>>,
    }

    impl CoordinationStore for FakeStore {
        fn scratchpads(&self, _project: &std::path::Path) -> Result<Vec<Scratchpad>, ConfigError> {
            Ok(self.notes.lock().unwrap().clone())
        }

        fn scratchpad(
            &self,
            _project: &std::path::Path,
            key: &ScratchpadKey,
        ) -> Result<Option<Scratchpad>, ConfigError> {
            Ok(self
                .notes
                .lock()
                .unwrap()
                .iter()
                .find(|note| note.key() == key)
                .cloned())
        }

        fn set_scratchpad(
            &self,
            _project: &std::path::Path,
            author: &Author,
            key: &ScratchpadKey,
            body: &str,
        ) -> Result<(), ConfigError> {
            let note = Scratchpad::builder()
                .key(key.clone())
                .body(body.to_owned())
                .author(author.clone())
                .updated_at(0)
                .build();
            let mut notes = self.notes.lock().unwrap();
            notes.retain(|existing| existing.key() != key);
            notes.push(note);
            Ok(())
        }

        fn delete_scratchpad(
            &self,
            _project: &std::path::Path,
            _key: &ScratchpadKey,
        ) -> Result<bool, ConfigError> {
            unreachable!("this test never deletes scratchpads")
        }

        fn todos(&self, _project: &std::path::Path) -> Result<Vec<Todo>, ConfigError> {
            unreachable!("this test never touches todos")
        }

        fn todo(
            &self,
            _project: &std::path::Path,
            _id: &TodoId,
        ) -> Result<Option<Todo>, ConfigError> {
            unreachable!("this test never touches todos")
        }

        fn add_todo(
            &self,
            _project: &std::path::Path,
            _author: &Author,
            _title: &TodoTitle,
            _deps: &[TodoId],
        ) -> Result<Todo, ConfigError> {
            unreachable!("this test never touches todos")
        }

        fn set_todo_status(
            &self,
            _project: &std::path::Path,
            _author: &Author,
            _id: &TodoId,
            _status: TodoStatus,
        ) -> Result<bool, ConfigError> {
            unreachable!("this test never touches todos")
        }

        fn delete_todo(
            &self,
            _project: &std::path::Path,
            _id: &TodoId,
        ) -> Result<bool, ConfigError> {
            unreachable!("this test never touches todos")
        }

        fn values(&self, _project: &std::path::Path) -> Result<Vec<KeyValue>, ConfigError> {
            unreachable!("this test never touches key-values")
        }

        fn value(
            &self,
            _project: &std::path::Path,
            _key: &KvKey,
        ) -> Result<Option<KeyValue>, ConfigError> {
            unreachable!("this test never touches key-values")
        }

        fn set_value(
            &self,
            _project: &std::path::Path,
            _author: &Author,
            _key: &KvKey,
            _value: &str,
        ) -> Result<(), ConfigError> {
            unreachable!("this test never touches key-values")
        }

        fn delete_value(
            &self,
            _project: &std::path::Path,
            _key: &KvKey,
        ) -> Result<bool, ConfigError> {
            unreachable!("this test never touches key-values")
        }

        fn import(
            &self,
            _project: &std::path::Path,
            _scratchpads: &[Scratchpad],
            _todos: &[Todo],
            _values: &[KeyValue],
        ) -> Result<(), ConfigError> {
            unreachable!("these tests never import")
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

    /// A `scratchpad_set` tool call writes through to the store, and
    /// `scratchpad_list` reads it back as structured data - the store-backed
    /// round trip an agent depends on.
    #[test]
    fn a_set_tool_call_writes_through_to_the_store() {
        let server = CoordinationMcp::new(
            Box::new(FakeStore::default()),
            PathBuf::from("/repo/muster.yml"),
            Author::agent("test"),
        );

        runtime().block_on(async {
            let saved = server
                .scratchpad_set(Parameters(ScratchpadSetArg {
                    key: "plan".to_string(),
                    body: "ship it".to_string(),
                }))
                .await
                .unwrap();
            assert_eq!(result_text(&saved), "saved 'plan'");

            // The list is an index: key + provenance, no bodies.
            let listed = server
                .scratchpad_list(Parameters(ListArg { limit: None }))
                .await
                .unwrap();
            let notes = listed
                .structured_content
                .expect("list returns structured data");
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

            // The body is fetched per key.
            let fetched = server
                .scratchpad_get(Parameters(KeyArg {
                    key: "plan".to_string(),
                }))
                .await
                .unwrap();
            assert_eq!(
                fetched.structured_content.expect("structured note")["body"],
                "ship it"
            );
        });
    }

    /// A blank key is rejected as invalid params before the store is touched.
    #[test]
    fn a_blank_key_is_invalid_params() {
        let server = CoordinationMcp::new(
            Box::new(FakeStore::default()),
            PathBuf::from("/repo/muster.yml"),
            Author::agent("test"),
        );

        runtime().block_on(async {
            let error = server
                .scratchpad_set(Parameters(ScratchpadSetArg {
                    key: "   ".to_string(),
                    body: "x".to_string(),
                }))
                .await
                .unwrap_err();
            assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        });
    }

    /// The connection snippet is valid JSON that launches this binary with the
    /// project's config path under the muster server key.
    #[test]
    fn connection_config_is_valid_and_points_at_this_binary() {
        let json = connection_config(
            Path::new("/usr/local/bin/muster"),
            Path::new("/repo/muster.yml"),
        );
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let entry = &parsed["mcpServers"][MCP_CONNECTION_KEY];
        assert_eq!(entry["command"], "/usr/local/bin/muster");
        assert_eq!(
            entry["args"],
            serde_json::json!(["mcp", "--config", "/repo/muster.yml"])
        );
    }

    /// `process_list` reaches a running workspace over its socket and returns the
    /// live roster as structured data. A fake workspace stands in for the TUI.
    #[cfg(unix)]
    #[test]
    fn process_list_returns_the_live_roster() {
        use std::thread;

        use crate::adapter::{
            bridge::{ProcessSummary, WireActivity, WireKind, WireState},
            ipc,
        };

        let project = std::env::temp_dir()
            .join(format!("muster-live-{}", uuid::Uuid::new_v4()))
            .join("muster.yml");
        let path = ipc::socket_path(&project).expect("a socket path");
        let listener = ipc::bind(&path).expect("bind the fake workspace socket");

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            ipc::serve_connection(stream, |request: WorkspaceRequest| {
                assert_eq!(request, WorkspaceRequest::ListProcesses);
                WorkspaceResponse::Processes(vec![
                    ProcessSummary::builder()
                        .id(1)
                        .name("api".to_string())
                        .kind(WireKind::Command)
                        .state(WireState::Running)
                        .activity(WireActivity::Working)
                        .build(),
                ])
            })
            .unwrap();
        });

        let mcp = CoordinationMcp::new(
            Box::new(FakeStore::default()),
            project,
            Author::agent("test"),
        );
        runtime().block_on(async {
            let result = mcp.process_list().await.unwrap();
            let roster = result.structured_content.expect("structured roster");
            assert_eq!(roster[0]["name"], "api");
            assert_eq!(roster[0]["state"], "running");
            assert_eq!(roster[0]["activity"], "working");
        });

        server.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }

    /// `process_output` reads a pane's text over the socket; an unknown id is an
    /// invalid-params error, and no workspace is a note.
    #[cfg(unix)]
    #[test]
    fn process_output_reads_a_pane_over_the_socket() {
        use std::thread;

        use crate::adapter::ipc;

        let project = std::env::temp_dir()
            .join(format!("muster-out-{}", uuid::Uuid::new_v4()))
            .join("muster.yml");
        let path = ipc::socket_path(&project).expect("a socket path");
        let listener = ipc::bind(&path).expect("bind the fake workspace socket");

        // The fake workspace answers pane 1 with text and any other id with None.
        let server = thread::spawn(move || {
            for _ in 0..2 {
                let (stream, _) = listener.accept().unwrap();
                ipc::serve_connection(stream, |request: WorkspaceRequest| match request {
                    WorkspaceRequest::PaneOutput { pane: 1 } => {
                        WorkspaceResponse::PaneOutput(Some("build passed".to_string()))
                    },
                    WorkspaceRequest::PaneOutput { .. } => WorkspaceResponse::PaneOutput(None),
                    other => panic!("expected a pane-output request, got {other:?}"),
                })
                .unwrap();
            }
        });

        let mcp = CoordinationMcp::new(
            Box::new(FakeStore::default()),
            project,
            Author::agent("test"),
        );
        runtime().block_on(async {
            let found = mcp
                .process_output(Parameters(PaneArg { id: 1 }))
                .await
                .unwrap();
            assert_eq!(result_text(&found), "build passed");

            let missing = mcp
                .process_output(Parameters(PaneArg { id: 9 }))
                .await
                .unwrap_err();
            assert_eq!(missing.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        });

        server.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }

    /// `process_send` writes to a pane over the socket and maps each outcome:
    /// delivered acks, not-running and unknown are invalid-params errors.
    #[cfg(unix)]
    #[test]
    fn process_send_maps_each_outcome() {
        use std::thread;

        use crate::adapter::{
            bridge::{SendOutcome, WorkspaceResponse},
            ipc,
        };

        let project = std::env::temp_dir()
            .join(format!("muster-send-{}", uuid::Uuid::new_v4()))
            .join("muster.yml");
        let path = ipc::socket_path(&project).expect("a socket path");
        let listener = ipc::bind(&path).expect("bind the fake workspace socket");

        // The fake workspace delivers to id 1, reports id 2 stopped, id 3 unknown.
        let server = thread::spawn(move || {
            for _ in 0..3 {
                let (stream, _) = listener.accept().unwrap();
                ipc::serve_connection(stream, |request: WorkspaceRequest| {
                    let WorkspaceRequest::SendInput { pane, data } = request else {
                        panic!("expected SendInput");
                    };
                    assert_eq!(data, "go\r");
                    let outcome = match pane {
                        1 => SendOutcome::Delivered,
                        2 => SendOutcome::NotRunning,
                        _ => SendOutcome::Unknown,
                    };
                    WorkspaceResponse::Sent(outcome)
                })
                .unwrap();
            }
        });

        let mcp = CoordinationMcp::new(
            Box::new(FakeStore::default()),
            project,
            Author::agent("test"),
        );
        runtime().block_on(async {
            let send = |id| {
                mcp.process_send(Parameters(SendInputArg {
                    id,
                    text: "go".to_string(),
                    enter: true,
                }))
            };
            assert_eq!(result_text(&send(1).await.unwrap()), "sent");
            assert_eq!(
                send(2).await.unwrap_err().code,
                rmcp::model::ErrorCode::INVALID_PARAMS
            );
            assert_eq!(
                send(3).await.unwrap_err().code,
                rmcp::model::ErrorCode::INVALID_PARAMS
            );
        });

        server.join().unwrap();
        let _ = std::fs::remove_file(&path);
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
        let server = CoordinationMcp::new(
            Box::new(FakeStore::default()),
            PathBuf::from("/repo/muster.yml"),
            Author::agent("test"),
        );
        runtime().block_on(async {
            for key in ["a", "b", "c"] {
                server
                    .scratchpad_set(Parameters(ScratchpadSetArg {
                        key: key.to_string(),
                        body: "x".to_string(),
                    }))
                    .await
                    .unwrap();
            }
            let listed = server
                .scratchpad_list(Parameters(ListArg { limit: Some(2) }))
                .await
                .unwrap();
            let notes = listed.structured_content.expect("structured index");
            assert_eq!(notes.as_array().expect("an array").len(), 2);
        });
    }

    /// The three lifecycle tools reach the workspace and map their outcome: a
    /// requested action acks, an unknown id is invalid-params.
    #[cfg(unix)]
    #[test]
    fn lifecycle_tools_request_actions_and_report_unknown_ids() {
        use std::thread;

        use crate::adapter::{
            bridge::{ControlAction, ControlOutcome, WorkspaceResponse},
            ipc,
        };

        let project = std::env::temp_dir()
            .join(format!("muster-ctl-{}", uuid::Uuid::new_v4()))
            .join("muster.yml");
        let path = ipc::socket_path(&project).expect("a socket path");
        let listener = ipc::bind(&path).expect("bind the fake workspace socket");

        // The fake workspace accepts id 1 and reports any other id unknown, and
        // records which action each call asked for.
        let server = thread::spawn(move || {
            let mut seen = Vec::new();
            for _ in 0..4 {
                let (stream, _) = listener.accept().unwrap();
                ipc::serve_connection(stream, |request: WorkspaceRequest| {
                    let WorkspaceRequest::ControlProcess { pane, action } = request else {
                        panic!("expected ControlProcess");
                    };
                    seen.push(action);
                    WorkspaceResponse::Controlled(if pane == 1 {
                        ControlOutcome::Requested
                    } else {
                        ControlOutcome::Unknown
                    })
                })
                .unwrap();
            }
            seen
        });

        let mcp = CoordinationMcp::new(
            Box::new(FakeStore::default()),
            project,
            Author::agent("test"),
        );
        runtime().block_on(async {
            assert_eq!(
                result_text(
                    &mcp.process_start(Parameters(PaneArg { id: 1 }))
                        .await
                        .unwrap()
                ),
                "requested start for process 1"
            );
            assert_eq!(
                result_text(
                    &mcp.process_stop(Parameters(PaneArg { id: 1 }))
                        .await
                        .unwrap()
                ),
                "requested stop for process 1"
            );
            assert_eq!(
                result_text(
                    &mcp.process_restart(Parameters(PaneArg { id: 1 }))
                        .await
                        .unwrap()
                ),
                "requested restart for process 1"
            );
            assert_eq!(
                mcp.process_start(Parameters(PaneArg { id: 9 }))
                    .await
                    .unwrap_err()
                    .code,
                rmcp::model::ErrorCode::INVALID_PARAMS
            );
        });

        let seen = server.join().unwrap();
        assert_eq!(
            seen,
            vec![
                ControlAction::Start,
                ControlAction::Stop,
                ControlAction::Restart,
                ControlAction::Start
            ],
            "each tool asks for its own action"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// `session_list` returns the workspace's durable agent sessions.
    #[cfg(unix)]
    #[test]
    fn session_list_returns_the_projects_sessions() {
        use std::thread;

        use crate::adapter::{
            bridge::{SessionSummary, WorkspaceResponse},
            ipc,
        };

        let project = std::env::temp_dir()
            .join(format!("muster-sess-{}", uuid::Uuid::new_v4()))
            .join("muster.yml");
        let path = ipc::socket_path(&project).expect("a socket path");
        let listener = ipc::bind(&path).expect("bind the fake workspace socket");

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            ipc::serve_connection(stream, |request: WorkspaceRequest| {
                assert_eq!(request, WorkspaceRequest::ListSessions);
                WorkspaceResponse::Sessions(vec![
                    SessionSummary::builder()
                        .name("Ada".to_string())
                        .tool("Claude".to_string())
                        .state("open".to_string())
                        .resumable(true)
                        .build(),
                ])
            })
            .unwrap();
        });

        let mcp = CoordinationMcp::new(
            Box::new(FakeStore::default()),
            project,
            Author::agent("test"),
        );
        runtime().block_on(async {
            let listed = mcp.session_list().await.unwrap();
            let sessions = listed.structured_content.expect("structured sessions");
            assert_eq!(sessions[0]["name"], "Ada");
            assert_eq!(sessions[0]["tool"], "Claude");
            assert_eq!(sessions[0]["state"], "open");
            assert_eq!(sessions[0]["resumable"], true);
        });

        server.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }

    /// With no workspace running for the project, `process_list` returns the
    /// no-workspace note rather than erroring.
    #[test]
    fn process_list_notes_when_no_workspace_runs() {
        let server = CoordinationMcp::new(
            Box::new(FakeStore::default()),
            PathBuf::from("/no/such/repo/muster.yml"),
            Author::agent("test"),
        );
        runtime().block_on(async {
            let result = server.process_list().await.unwrap();
            assert_eq!(result_text(&result), NO_WORKSPACE);
        });
    }

    /// The advertised server info names the coordination server and enables the
    /// tools capability.
    #[test]
    fn server_info_advertises_tools() {
        let server = CoordinationMcp::new(
            Box::new(FakeStore::default()),
            PathBuf::from("/repo/muster.yml"),
            Author::agent("test"),
        );
        let info = server.get_info();
        assert_eq!(info.server_info.name, MCP_SERVER_NAME);
        assert!(info.capabilities.tools.is_some());
    }
}
