use std::path::PathBuf;

use crate::{
    adapter::{
        holder::{MarksStamp, marks_stamp, stream_marks},
        terminal::placed_renderer::PlacedRenderer,
    },
    domain::value::OutputCursor,
};

/// The marks of a followed log, read again only when the sidecar changed,
/// so an idle follow costs a `stat` per poll, not a read and parse.
pub struct MarksWatch {
    /// The log whose marks are watched, when there is one.
    log: Option<PathBuf>,
    /// The sidecar's size and time when it was last read.
    seen: Option<MarksStamp>,
}

impl MarksWatch {
    /// Watches the marks of the log at `log`.
    pub fn new(log: Option<PathBuf>) -> Self {
        Self { log, seen: None }
    }

    /// A renderer whose first byte sits at `start`, in the state the marks
    /// say the stream is in there.
    pub fn renderer_at(&mut self, start: OutputCursor) -> PlacedRenderer {
        let Some(log) = &self.log else {
            return PlacedRenderer::new(start);
        };
        self.seen = marks_stamp(log);
        PlacedRenderer::placed(start, &stream_marks(log))
    }

    /// Notes on `renderer` the run starts recorded since the marks were
    /// last read, once the sidecar changed.
    pub fn refresh(&mut self, renderer: &mut PlacedRenderer) {
        let Some(log) = &self.log else {
            return;
        };
        let stamp = marks_stamp(log);
        if stamp == self.seen {
            return;
        }
        self.seen = stamp;
        renderer.restart_at(stream_marks(log).run_starts());
    }
}
