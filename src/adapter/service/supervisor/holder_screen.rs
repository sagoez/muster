use std::sync::Arc;

use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::spawn_generation::SpawnGeneration;
use crate::domain::{
    port::ScreenReader,
    pty::PtyError,
    value::{PaneId, ProjectRoot},
};

/// The screen of a process served elsewhere, still to be read. The read can
/// block, so the supervisor hands it to its caller instead of reading it on
/// its own loop.
#[derive(Debug, Clone, Getters, CopyGetters, TypedBuilder)]
pub struct HolderScreen {
    /// The project the pane belonged to when the read was handed out.
    #[getset(get = "pub")]
    project: ProjectRoot,
    /// The pane whose screen this is.
    #[getset(get_copy = "pub")]
    pane: PaneId,
    /// The pane's spawn generation when the read was handed out, so a read
    /// that outlived its process is told apart.
    #[getset(get_copy = "pub")]
    generation: SpawnGeneration,
    /// Reads the screen from where it is served.
    #[getset(get = "pub")]
    reader: Arc<dyn ScreenReader>,
    /// The last screen the service saw of the process, used when the holder
    /// cannot be read.
    #[getset(get = "pub")]
    fallback: String,
}

impl HolderScreen {
    /// Reads the visible screen from where it is served.
    ///
    /// # Errors
    /// Returns a [`PtyError`] when the screen cannot be reached or read.
    pub fn read(&self) -> Result<String, PtyError> {
        self.reader.read_screen()
    }
}
