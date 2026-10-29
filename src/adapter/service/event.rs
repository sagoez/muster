use std::collections::HashMap;

use crossbeam_channel::Sender;

use super::{
    probe_target::ProbeTarget,
    service_request::ServiceRequest,
    service_response::ServiceResponse,
    supervisor::{HolderScreen, ShutdownGeneration, SpawnGeneration},
};
use crate::{
    adapter::holder::HolderRecord,
    domain::{
        port::{ListeningSocket, ProcessHandle},
        pty::{ProcessOutput, PtyError},
        value::{PaneId, Pid, ProjectRoot},
    },
};

/// A message for the service's event loop from a process, a timer, a worker
/// or a client.
pub enum ServiceEvent {
    /// Output from a running process.
    Output {
        /// Project the process belongs to.
        project: ProjectRoot,
        /// Pane that produced the output.
        pane: PaneId,
        /// Spawn generation that produced it; output from a superseded
        /// generation is ignored.
        generation: SpawnGeneration,
        /// The output event.
        output: ProcessOutput,
    },
    /// A pane's restart backoff has elapsed; respawn it if still current.
    Respawn {
        /// Project the process belongs to.
        project: ProjectRoot,
        /// Pane to respawn.
        pane: PaneId,
        /// Generation captured when the restart was scheduled; a mismatch means
        /// a later action superseded it.
        generation: SpawnGeneration,
    },
    /// A graceful stop deadline elapsed; force the stop if the same process and
    /// stop request are still current.
    ForceStop {
        /// Project the process belongs to.
        project: ProjectRoot,
        /// Pane whose process should be forcibly stopped.
        pane: PaneId,
        /// Spawn generation captured when graceful termination began.
        spawn_generation: SpawnGeneration,
        /// Stop request generation captured when its deadline began.
        shutdown_generation: ShutdownGeneration,
    },
    /// A launch handed to a worker finished; install the process or settle
    /// the failure, unless a later action superseded the launch.
    Spawned {
        /// Project the process belongs to.
        project: ProjectRoot,
        /// Pane the process was launched for.
        pane: PaneId,
        /// Spawn generation the launch belongs to.
        generation: SpawnGeneration,
        /// The running process, or why it could not be launched.
        result: Result<Box<dyn ProcessHandle>, PtyError>,
    },
    /// An adoption handed to a worker finished; supervise the holder or settle
    /// the failure, unless a later action superseded it.
    Adopted {
        /// Project the process belongs to.
        project: ProjectRoot,
        /// Pane the holder was adopted for.
        pane: PaneId,
        /// Spawn generation the adoption belongs to.
        generation: SpawnGeneration,
        /// The record of the holder that was adopted, boxed so this rare
        /// event does not size every other one.
        record: Box<HolderRecord>,
        /// The adopted process, or why its holder could not be adopted.
        result: Result<Box<dyn ProcessHandle>, PtyError>,
    },
    /// A port or HTTP probe passed for one run of a process; a pass for a run
    /// that is no longer current is ignored.
    ProbePassed {
        /// Project the process belongs to.
        project: ProjectRoot,
        /// Pane of the process.
        pane: PaneId,
        /// The run the probe was for.
        generation: SpawnGeneration,
        /// What passed.
        probe: ProbeTarget,
    },
    /// The ports each running process tree listens on, keyed by root pid.
    PortsSampled {
        /// The sockets of each root pid that has any.
        ports: HashMap<Pid, Vec<ListeningSocket>>,
    },
    /// A client's request, to be answered on `reply`.
    Request {
        /// What the client asks.
        request: ServiceRequest,
        /// Where the answer goes; dropping it unanswered tells the client the
        /// service is going away.
        reply: Sender<ServiceResponse>,
    },
    /// A holder screen was read off the loop; settle it and answer the
    /// request that asked for it.
    HolderRead {
        /// The screen that was read.
        screen: HolderScreen,
        /// The text read, or why the holder could not be read.
        read: Result<String, PtyError>,
        /// Where the answer goes.
        reply: Sender<ServiceResponse>,
    },
}
