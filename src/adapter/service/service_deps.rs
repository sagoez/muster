use std::{path::PathBuf, sync::Arc};

use typed_builder::TypedBuilder;

use super::{
    holder_pool::HolderPool, project_ports::ProjectPorts, service_version::ServiceVersion,
};
use crate::domain::port::{
    ListeningPorts, ProcessMetrics, RaisedEdits, RunHistory, SettingsReader, StackEditLog,
    StackReader,
};

/// What the service runs on. No `Getters`: the service takes it apart once.
#[derive(TypedBuilder)]
pub struct ServiceDeps {
    /// The running binary's version, answered to a client's hello.
    pub(super) version: ServiceVersion,
    /// The running binary's path, answered to a client's hello so an older
    /// client can say which muster it found.
    pub(super) binary: PathBuf,
    /// The socket clients reach the service at.
    pub(super) socket: PathBuf,
    /// The holders every project's processes run under.
    pub(super) holders: Arc<dyn HolderPool>,
    /// Builds each project's driven adapters as it is loaded.
    pub(super) ports: Box<dyn ProjectPorts>,
    /// Samples the resource usage of every project's processes in one pass;
    /// absent where usage is not reported.
    pub(super) metrics: Option<Box<dyn ProcessMetrics>>,
    /// Reads each project's stack as it is loaded and as it changes.
    pub(super) stacks: Box<dyn StackReader>,
    /// Reads the user's settings at start and as they change.
    pub(super) settings: Box<dyn SettingsReader>,
    /// Reads the agent edits each project's stack took, raised as events.
    pub(super) edits: Box<dyn StackEditLog>,
    /// Keeps how each process last ended on its own, read as a project loads
    /// and written off the loop as runs end.
    pub(super) runs: Arc<dyn RunHistory>,
    /// Keeps the newest agent edit of each project raised as an event, read
    /// as a project loads and written off the loop.
    pub(super) raised: Arc<dyn RaisedEdits>,
    /// Finds the ports running process trees listen on; absent where ports
    /// are not reported.
    pub(super) listening: Option<Box<dyn ListeningPorts + Send>>,
}
