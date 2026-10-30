use std::{path::PathBuf, sync::Arc};

use signal_hook::iterator::Signals;
use typed_builder::TypedBuilder;

use super::{
    holder_pool::HolderPool, project_ports::ProjectPorts, service_version::ServiceVersion,
};
use crate::domain::port::{
    ListeningPorts, PendingIdleStops, ProcessMetrics, RaisedEdits, RunHistory, SettingsReader,
    ShellRegistry, StackEditLog, StackReader,
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
    /// Reads how each process last ended on its own, as a project loads.
    pub(super) runs: Box<dyn RunHistory>,
    /// Reads the newest agent edit of each project raised as an event, as a
    /// project loads.
    pub(super) raised: Box<dyn RaisedEdits>,
    /// Writes how runs end, off the loop, on a database connection of its
    /// own, so its wait for another writer never holds the loop's.
    pub(super) run_writes: Box<dyn RunHistory>,
    /// Writes which agent edits were raised, on the same connection as
    /// `run_writes`.
    pub(super) raised_writes: Box<dyn RaisedEdits>,
    /// Reads the idle stops pending when a service last ran, at start.
    pub(super) idle_stops: Box<dyn PendingIdleStops>,
    /// Writes which idle stops are pending, on the same connection as
    /// `run_writes`.
    pub(super) idle_stop_writes: Box<dyn PendingIdleStops>,
    /// Finds the ports running process trees listen on; absent where ports
    /// are not reported.
    pub(super) listening: Option<Box<dyn ListeningPorts + Send>>,
    /// The shells running muster's hook and the last report each made, read
    /// at start to know who is in which project; ended ones are forgotten.
    pub(super) shells: Arc<dyn ShellRegistry>,
    /// The signals that stop the service as a client's stop does; absent
    /// for a service run inside another program, such as a test.
    pub(super) exit_signals: Option<Signals>,
}
