use crossbeam_channel::Sender;

use super::spawn_generation::SpawnGeneration;
use crate::{
    adapter::service::ServiceEvent,
    domain::{
        port::OutputSink,
        pty::ProcessOutput,
        value::{PaneId, ProjectRoot},
    },
};

/// An [`OutputSink`] that forwards a process's output onto the service event
/// channel, tagged with its project, pane and spawn generation.
pub struct ChannelOutputSink {
    project: ProjectRoot,
    pane: PaneId,
    generation: SpawnGeneration,
    sender: Sender<ServiceEvent>,
}

impl ChannelOutputSink {
    /// Creates a sink that tags output with `project`, `pane` and its spawn
    /// `generation` and sends it on `sender`.
    pub fn new(
        project: ProjectRoot,
        pane: PaneId,
        generation: SpawnGeneration,
        sender: Sender<ServiceEvent>,
    ) -> Self {
        Self {
            project,
            pane,
            generation,
            sender,
        }
    }
}

impl OutputSink for ChannelOutputSink {
    fn send(&self, output: ProcessOutput) {
        let _ = self.sender.send(ServiceEvent::Output {
            project: self.project.clone(),
            pane: self.pane,
            generation: self.generation,
            output,
        });
    }
}
