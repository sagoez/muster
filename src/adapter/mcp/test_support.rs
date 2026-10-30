//! A server over a scratch database whose service is never reached unless a
//! test hands it a scratch one, so no test touches the user's real state,
//! service socket or terminal workspace.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use getset::{Getters, MutGetters};
use rmcp::{
    ErrorData,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock},
};
use serde_json::Value;
use tempfile::TempDir;
use tokio::runtime::{Builder, Runtime};

use super::{
    mcp_deps::McpDeps, process_add_arg::ProcessAddArg, scope::Scope, server::MusterMcp,
    tool_failure::ToolFailure,
};
use crate::{
    adapter::{
        client::{fake_service::FakeService, recording_notifier::RecordingNotifier},
        store::SqliteStore,
    },
    domain::{
        port::{StackEditLog, TerminalWorkspace},
        value::ProjectRoot,
    },
};

/// The database file each test's store opens.
const DATABASE: &str = "muster.db";
/// The project folder each test serves, inside its temporary directory.
const PROJECT_DIR: &str = "shop";
/// The `--as` name each test server writes under.
pub const AGENT: &str = "test";
/// The stand-in muster binary each test server's panes would run, inside
/// its temporary directory.
const BINARY: &str = "muster";

/// A server started in a folder no project claims yet, with the store and
/// notifier it writes through.
#[derive(Getters, MutGetters)]
pub struct Fixture {
    /// Holds the scratch directory for the fixture's life.
    _dir: TempDir,
    /// The folder the server was started in.
    #[getset(get = "pub", get_mut = "pub")]
    root: ProjectRoot,
    /// The database.
    #[getset(get = "pub", get_mut = "pub")]
    store: SqliteStore,
    /// What the server told about its edits.
    #[getset(get = "pub", get_mut = "pub")]
    notifier: Arc<RecordingNotifier>,
    /// The server.
    #[getset(get = "pub", get_mut = "pub")]
    server: MusterMcp,
    /// The binary the server reads at each open, which a test may change.
    binary: Arc<Mutex<PathBuf>>,
}

impl Fixture {
    /// The binary the server's next open reads.
    pub fn binary(&self) -> PathBuf {
        self.binary.lock().unwrap().clone()
    }

    /// Makes the server's next open read `binary`.
    pub fn use_binary(&self, binary: PathBuf) {
        *self.binary.lock().unwrap() = binary;
    }
}

/// A [`Fixture`] whose folder, holding a `frontend` folder, is not yet a
/// project, and whose server reaches no service and runs in no terminal
/// workspace.
pub fn fixture() -> Fixture {
    fixture_with(FakeService::default(), None)
}

/// A [`Fixture`] as [`fixture`] makes it, whose server drives `service` and
/// runs in `workspace`, if any; its panes would run a stand-in binary that
/// exists.
pub fn fixture_with(
    service: FakeService,
    workspace: Option<Arc<dyn TerminalWorkspace>>,
) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join(PROJECT_DIR);
    fs::create_dir_all(folder.join("frontend")).unwrap();
    let root = ProjectRoot::try_new(fs::canonicalize(&folder).unwrap()).unwrap();
    let store = SqliteStore::open(&dir.path().join(DATABASE)).unwrap();
    let notifier = Arc::new(RecordingNotifier::default());
    let binary_path = dir.path().join(BINARY);
    fs::write(&binary_path, "").unwrap();
    let binary = Arc::new(Mutex::new(binary_path));
    let server = serving(&root, &store, &notifier, AGENT, service, workspace, &binary);
    Fixture {
        _dir: dir,
        root,
        store,
        notifier,
        server,
        binary,
    }
}

/// A second server in `fixture`'s folder and database, writing as `agent`,
/// reaching no service and running in no terminal workspace.
pub fn server_as(fixture: &Fixture, agent: &str) -> MusterMcp {
    serving(
        &fixture.root,
        &fixture.store,
        &fixture.notifier,
        agent,
        FakeService::default(),
        None,
        &fixture.binary,
    )
}

/// A server started in `root` over `store`, telling `notifier` of its edits,
/// writing as `agent`, driving `service`, running in `workspace` and
/// reading its binary from `binary` at each open.
fn serving(
    root: &ProjectRoot,
    store: &SqliteStore,
    notifier: &Arc<RecordingNotifier>,
    agent: &str,
    service: FakeService,
    workspace: Option<Arc<dyn TerminalWorkspace>>,
    binary: &Arc<Mutex<PathBuf>>,
) -> MusterMcp {
    let binary = binary.clone();
    let deps = McpDeps::builder()
        .scope(
            Scope::builder()
                .cwd(root.as_ref().clone())
                .base(root.as_ref().clone())
                .home(root.as_ref().parent().map(Path::to_path_buf))
                .build(),
        )
        .agent(Some(agent.to_string()))
        .service(Arc::new(service))
        .projects(Arc::new(store.clone()))
        .coordination(Arc::new(store.clone()))
        .stacks(Arc::new(store.clone()))
        .settings(Arc::new(store.clone()))
        .edits(Arc::new(store.clone()))
        .notifier(notifier.clone())
        .terminal_workspace(workspace)
        .binary(Arc::new(move || binary.lock().unwrap().clone()))
        .build();
    MusterMcp::new(deps)
}

/// Each agent edit the fixture's project log keeps, as the user reads it,
/// oldest first.
pub fn edits(fixture: &Fixture) -> Vec<String> {
    fixture
        .store
        .edits(&fixture.root)
        .unwrap()
        .iter()
        .map(|edit| edit.message().clone())
        .collect()
}

/// `process_add` arguments for `name` running `command`.
pub fn add(name: &str, command: &str) -> ProcessAddArg {
    ProcessAddArg::builder()
        .name(name.to_string())
        .command(command.to_string())
        .build()
}

/// Registers the fixture's folder by adding a `web` process to it.
pub fn register(fixture: &Fixture) {
    runtime().block_on(async {
        content(
            fixture
                .server
                .process_add(Parameters(add("web", "npm run dev")))
                .await,
        );
    });
}

/// Builds a single-threaded runtime for the async tool calls.
pub fn runtime() -> Runtime {
    Builder::new_current_thread().enable_time().build().unwrap()
}

/// The text of a tool result's first content block.
pub fn result_text(result: &CallToolResult) -> String {
    match &result.content[0] {
        ContentBlock::Text(text) => text.text.clone(),
        other => panic!("expected text content, got {other:?}"),
    }
}

/// What the agent receives for `result`.
pub fn answered(result: Result<CallToolResult, ToolFailure>) -> Result<CallToolResult, ErrorData> {
    result.or_else(ToolFailure::answer)
}

/// The text of the error result an agent receives for `result`.
pub fn refusal(result: Result<CallToolResult, ToolFailure>) -> String {
    let answer = answered(result).expect("a result, not a protocol error");
    assert_eq!(answer.is_error, Some(true));
    result_text(&answer)
}

/// The structured content of the successful result `result`.
pub fn content(result: Result<CallToolResult, ToolFailure>) -> Value {
    let answer = answered(result).expect("a result, not a protocol error");
    assert_ne!(answer.is_error, Some(true), "{:?}", answer.content);
    answer.structured_content.expect("structured content")
}
