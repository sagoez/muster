use std::path::PathBuf;

use crossbeam_channel::Sender;
use crossterm::event::Event as CrosstermEvent;

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
        generation: u64,
        /// The output event.
        output: ProcessOutput,
    },
    /// A pane's restart backoff has elapsed; respawn it if still current.
    Respawn {
        /// Pane to respawn.
        pane: PaneId,
        /// Generation captured when the restart was scheduled; a mismatch means
        /// a later action superseded it and the respawn must be ignored.
        generation: u64,
    },
    /// Directory-autocomplete candidates computed off the event loop.
    Completions {
        /// Request generation; a mismatch means a later edit superseded it and
        /// the candidates must be ignored.
        generation: u64,
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
    generation: u64,
    sender: Sender<RuntimeEvent>,
}

impl ChannelOutputSink {
    /// Creates a sink that tags output with `pane` and its spawn `generation`
    /// and sends it on `sender`.
    pub fn new(pane: PaneId, generation: u64, sender: Sender<RuntimeEvent>) -> Self {
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
