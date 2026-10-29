use std::{path::Path, sync::Arc, time::Duration};

use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt,
    handler::server::router::tool::ToolRouter,
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo},
    service::NotificationContext,
    tool_handler,
    transport::stdio,
};
use serde::Serialize;
use serde_json::{Map, Value};
use tokio::task;

use super::{
    alerts::{ALERT_KINDS, Alerts},
    alerts_view::AlertsView,
    described::{described, resolved},
    identity::Identity,
    mcp_deps::McpDeps,
    mcp_error::McpError,
    tool_error::ToolError,
    tool_failure::ToolFailure,
    tool_result::ToolResult,
};
use crate::{
    adapter::client::{ServiceClient, process},
    domain::project::Project,
};

/// Server name reported to connecting agents.
const MCP_SERVER_NAME: &str = "muster";
/// Key muster's server is registered under in an agent's MCP config; it
/// namespaces the tools an agent sees.
const MCP_CONNECTION_KEY: &str = "muster";
/// Subcommand an agent invokes to start the server.
const MCP_SUBCOMMAND: &str = "mcp";
/// Server version reported to connecting agents.
const MCP_SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Guidance shown to agents that connect to the server.
const MCP_INSTRUCTIONS: &str = "muster runs this project's dev stack (servers, workers, \
    watchers) and shares coordination state between agents. Typical loop: stack_status to see \
    what runs; after a code change, process_restart with wait=true; when something breaks, \
    process_logs with errors_only=true, then follow up with since=cursor to read only new \
    output. Results carry alerts when a process crashed since your last call. Before work other \
    agents must not repeat, take a lock (lock_acquire) or claim a todo (todo_claim). Calls never \
    block more than <call-bound-s> seconds; a wait that has not finished returns status waiting with a \
    cursor to continue from. Tools default to the project containing your working directory; \
    pass project to reach another.";
/// Key of the author's label echoed on a coordination write.
const YOU: &str = "you";
/// Opens each alert line appended to a rendered result's text.
const ALERT_LINE_PREFIX: &str = "[alert]";

/// The guidance shown to connecting agents, with its bounds resolved.
pub(super) fn instructions() -> String {
    resolved(MCP_INSTRUCTIONS)
}

/// Builds the pretty-printed JSON snippet an agent pastes into its
/// MCP-servers config to launch muster's server. It names no project, so one
/// entry serves every project. `executable` is the muster binary path, so
/// the entry works even when muster is not on `PATH`.
pub fn connection_config(executable: &Path) -> String {
    let value = serde_json::json!({
        "mcpServers": {
            MCP_CONNECTION_KEY: {
                "command": executable.display().to_string(),
                "args": [MCP_SUBCOMMAND],
            },
        },
    });
    serde_json::to_string_pretty(&value).unwrap_or_default()
}

/// Serves `deps` as an MCP server over stdio, blocking until the client
/// disconnects, then releases the locks this connection took; a crash
/// leaves them to lapse. Intended to run as a child process an agent spawns,
/// so the OS process boundary is the trust boundary.
///
/// # Errors
/// Returns [`McpError`] if the server cannot be initialized or stops abnormally.
pub(super) async fn serve_stdio(deps: McpDeps) -> Result<(), McpError> {
    let server = MusterMcp::new(deps.clone());
    let identity = server.identity.clone();
    let service = server
        .serve(stdio())
        .await
        .map_err(|error| McpError::Serve(error.to_string()))?;
    service
        .waiting()
        .await
        .map_err(|error| McpError::Wait(error.to_string()))?;
    release_session_locks(&deps, &identity);
    Ok(())
}

/// Releases the locks taken through `identity`'s session, and only those,
/// as its connection ends. They lapse by their TTL when the release fails.
pub(super) fn release_session_locks(deps: &McpDeps, identity: &Identity) {
    let _ = deps.coordination().release_locks_of(identity.session());
}

/// MCP driving adapter exposing projects' stacks, processes, settings and
/// coordination state as tools, each call scoped to the project it names or
/// the one containing the server's folder.
pub struct MusterMcp {
    /// The stores, service and scope the tools work through.
    pub(super) deps: McpDeps,
    /// Who this connection writes as.
    pub(super) identity: Arc<Identity>,
    /// The events this connection has been told about.
    pub(super) alerts: Arc<Alerts>,
    /// Every tool, from the stack, process, coordination and config routers.
    pub(super) tool_router: ToolRouter<Self>,
}

impl MusterMcp {
    /// Builds the server over `deps`.
    pub fn new(deps: McpDeps) -> Self {
        let identity = Arc::new(Identity::new(deps.agent().clone()));
        Self {
            deps,
            identity,
            alerts: Arc::new(Alerts::default()),
            tool_router: described(
                Self::stack_router()
                    + Self::process_router()
                    + Self::coordination_router()
                    + Self::config_router(),
            ),
        }
    }

    /// Runs `work` over this server's deps on a blocking thread, since it may
    /// write the database, reach the service or wait for a process.
    ///
    /// # Errors
    /// Returns the caller's mistake, such as an unknown name, or a fault of
    /// muster's.
    pub(super) async fn blocking<T, Work>(&self, work: Work) -> Result<T, ToolFailure>
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

    /// Runs `query` against the project `project` names through the service,
    /// starting or replacing the service as the CLI does, and reads the
    /// alerts this connection has not heard yet.
    ///
    /// # Errors
    /// Returns the caller's mistake, such as an unknown project or name, or
    /// the fault of a service that cannot be reached.
    pub(super) async fn ask<T, Query>(
        &self,
        project: Option<String>,
        query: Query,
    ) -> Result<(T, AlertsView), ToolFailure>
    where
        T: Send + 'static,
        Query: FnOnce(&ServiceClient, &Project) -> Result<T, ToolError> + Send + 'static,
    {
        let alerts = self.alerts.clone();
        self.blocking(move |deps| {
            let project = deps.project(project.as_deref())?;
            let client = deps.service().connect()?;
            let answer = query(&client, &project)?;
            Ok((answer, unheard(&alerts, &client, &project)))
        })
        .await
    }

    /// `value` with the author's label added the first time a write result
    /// carries it.
    pub(super) fn introduced(&self, value: Value) -> Value {
        match self.identity.introduce() {
            Some(label) => with_field(value, YOU, Value::String(label)),
            None => value,
        }
    }
}

/// The alerts of `project` this connection has not heard, reading events
/// without waiting. A failed read leaves them for the next call.
pub(super) fn unheard(alerts: &Alerts, client: &ServiceClient, project: &Project) -> AlertsView {
    let since = alerts.since(project.root());
    match process::events(client, project, since, &ALERT_KINDS, Duration::ZERO) {
        Ok(page) => alerts.take(project.root(), &page),
        Err(_) => AlertsView::default(),
    }
}

/// `value` serialized, which a tool's views always are.
///
/// # Errors
/// Returns an internal error if the value cannot be serialized.
pub(super) fn json<T: Serialize>(value: &T) -> Result<Value, ToolFailure> {
    serde_json::to_value(value)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None).into())
}

/// `value` with `field` set to `field_value`, when it is an object.
pub(super) fn with_field(value: Value, field: &str, field_value: Value) -> Value {
    match value {
        Value::Object(mut object) => {
            object.insert(field.to_string(), field_value);
            Value::Object(object)
        },
        other => other,
    }
}

/// `value` with the fields of `alerts` merged in.
///
/// # Errors
/// Returns an internal error if the alerts cannot be serialized.
pub(super) fn with_alerts(value: Value, alerts: &AlertsView) -> Result<Value, ToolFailure> {
    let Value::Object(fields) = json(alerts)? else {
        return Ok(value);
    };
    Ok(fields
        .into_iter()
        .fold(value, |value, (key, field)| with_field(value, &key, field)))
}

/// A structured result of `value`, flagged as an error when `failed`.
pub(super) fn structured(value: Value, failed: bool) -> ToolResult {
    Ok(if failed {
        CallToolResult::structured_error(value)
    } else {
        CallToolResult::structured(value)
    })
}

/// A result whose content is the compact `text` muster wrote, followed by
/// one line per alert, and whose structured content is `value`.
pub(super) fn rendered(
    text: String,
    value: Value,
    alerts: &AlertsView,
    failed: bool,
) -> ToolResult {
    let mut result = structured(with_alerts(value, alerts)?, failed)?;
    let mut text = text;
    for alert in alerts.alerts() {
        text.push_str(&format!(
            "\n{ALERT_LINE_PREFIX} {} {} {}",
            alert.process().as_deref().unwrap_or_default(),
            alert.kind(),
            alert.detail()
        ));
    }
    result.content = vec![ContentBlock::text(text)];
    Ok(result)
}

/// An empty JSON object, to build a result from.
pub(super) fn object() -> Value {
    Value::Object(Map::new())
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for MusterMcp {
    fn get_info(&self) -> ServerInfo {
        let mut implementation = Implementation::default();
        implementation.name = MCP_SERVER_NAME.to_string();
        implementation.version = MCP_SERVER_VERSION.to_string();
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = implementation;
        info.instructions = Some(instructions());
        info
    }

    async fn on_initialized(&self, context: NotificationContext<RoleServer>) {
        let client = context
            .peer
            .peer_info()
            .map(|info| info.client_info.name.clone());
        self.identity.settle(client.as_deref());
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::adapter::mcp::test_support::fixture;

    /// The connection snippet is valid JSON that launches this binary with
    /// no project, so one entry serves every project.
    #[test]
    fn connection_config_names_no_project() {
        let json = connection_config(&PathBuf::from("/usr/local/bin/muster"));
        let parsed: Value = serde_json::from_str(&json).unwrap();
        let entry = &parsed["mcpServers"][MCP_CONNECTION_KEY];
        assert_eq!(entry["command"], "/usr/local/bin/muster");
        assert_eq!(entry["args"], serde_json::json!(["mcp"]));
    }

    /// The server advertises its tools and every tool the design names.
    #[test]
    fn every_tool_is_served() {
        let fixture = fixture();
        let info = fixture.server.get_info();
        assert_eq!(info.server_info.name, MCP_SERVER_NAME);
        assert!(info.capabilities.tools.is_some());
        let mut names: Vec<String> = fixture
            .server
            .tool_router
            .list_all()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect();
        names.sort();
        assert_eq!(names, vec![
            "config_get",
            "config_set",
            "kv_delete",
            "kv_read",
            "kv_write",
            "lock_acquire",
            "lock_release",
            "note_delete",
            "note_read",
            "note_write",
            "process_add",
            "process_logs",
            "process_remove",
            "process_restart",
            "process_screen",
            "process_send",
            "process_set",
            "process_start",
            "process_stop",
            "process_wait",
            "stack_events",
            "stack_status",
            "stack_up",
            "todo_add",
            "todo_claim",
            "todo_delete",
            "todo_list",
            "todo_update",
        ]);
    }

    /// Alerts merge into a result's object, and a rendered result keeps
    /// muster's text as its content.
    #[test]
    fn alerts_merge_and_rendered_text_is_the_content() {
        let merged =
            with_alerts(serde_json::json!({ "sent": true }), &AlertsView::default()).unwrap();
        assert_eq!(merged, serde_json::json!({ "sent": true, "alerts": [] }));

        let result = rendered(
            "api o:0..o:5 (1 lines)\nhi".to_string(),
            object(),
            &AlertsView::default(),
            false,
        )
        .unwrap();
        let ContentBlock::Text(text) = &result.content[0] else {
            panic!("a text block");
        };
        assert_eq!(text.text, "api o:0..o:5 (1 lines)\nhi");
        assert_eq!(
            result.structured_content,
            Some(serde_json::json!({ "alerts": [] }))
        );
    }

    /// The first write result says who the agent is; later ones do not.
    #[test]
    fn the_first_write_introduces_the_agent() {
        let fixture = fixture();
        let first = fixture.server.introduced(object());
        assert_eq!(first[YOU], "test");
        assert!(fixture.server.introduced(object()).get(YOU).is_none());
    }
}
