use std::path::PathBuf;

use getset::Getters;
use typed_builder::TypedBuilder;

/// What `muster mcp` asks the composition root to serve: where each call
/// resolves its project from and the identity it was given.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct McpLaunch {
    /// The folder the server was started in.
    cwd: PathBuf,
    /// The folder a call naming no project resolves from: the `--project`
    /// root when one was given, the current folder otherwise.
    base: PathBuf,
    /// The `--as` name, which beats every other source of the agent's id.
    agent: Option<String>,
}
