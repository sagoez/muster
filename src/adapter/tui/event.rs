use std::path::PathBuf;

use crossbeam_channel::Sender;
use crossterm::event::Event as CrosstermEvent;

use super::{
    completion_generation::CompletionGeneration, shutdown_generation::ShutdownGeneration,
    spawn_generation::SpawnGeneration,
};
use crate::domain::{port::OutputSink, pty::ProcessOutput, value::PaneId};

/// A message processed by the runtime's event loop.
pub enum RuntimeEvent {
    /// A terminal input event from crossterm.
    Input(CrosstermEvent),
    /// Output from a running process.
    Output {
        /// Pane that produced the output.
        pane: PaneId,
        /// Spawn generation that produced it; the runtime ignores output from a
        /// superseded generation.
        generation: SpawnGeneration,
        /// The output event.
        output: ProcessOutput,
    },
    /// A pane's restart backoff has elapsed; respawn it if still current.
    Respawn {
        /// Pane to respawn.
        pane: PaneId,
        /// Generation captured when the restart was scheduled; a mismatch means
        /// a later action superseded it and the respawn must be ignored.
        generation: SpawnGeneration,
    },
    /// A graceful command-stop deadline elapsed; forcibly stop it if the same
    /// process and shutdown request are still current.
    ForceStop {
        /// Pane whose command should be forcibly stopped.
        pane: PaneId,
        /// Spawn generation captured when graceful termination began.
        spawn_generation: SpawnGeneration,
        /// Shutdown request generation captured when its deadline began.
        shutdown_generation: ShutdownGeneration,
    },
    /// Directory-autocomplete candidates computed off the event loop.
    Completions {
        /// Request generation; a mismatch means a later edit superseded it and
        /// the candidates must be ignored.
        generation: CompletionGeneration,
        /// The matching subdirectories to show.
        candidates: Vec<String>,
    },
    /// The active project's config file changed on disk (an edit, or a `muster`
    /// CLI append); the app should reconcile its process list.
    ConfigChanged {
        /// Normalized path of the config that changed.
        path: PathBuf,
    },
    /// The terminal input source failed or closed; the runtime should stop.
    InputClosed,
}

/// An [`OutputSink`] that forwards a process's output into the runtime channel,
/// tagged with the owning pane. The channel is bounded, so a noisy process
/// back-pressures here instead of growing memory without bound.
pub struct ChannelOutputSink {
    pane: PaneId,
    generation: SpawnGeneration,
    sender: Sender<RuntimeEvent>,
}

impl ChannelOutputSink {
    /// Creates a sink that tags output with `pane` and its spawn `generation`
    /// and sends it on `sender`.
    pub fn new(pane: PaneId, generation: SpawnGeneration, sender: Sender<RuntimeEvent>) -> Self {
        Self {
            pane,
            generation,
            sender,
        }
    }
}

impl OutputSink for ChannelOutputSink {
    fn send(&self, output: ProcessOutput) {
        let _ = self.sender.send(RuntimeEvent::Output {
            pane: self.pane,
            generation: self.generation,
            output,
        });
    }
}
