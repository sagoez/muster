use std::path::PathBuf;

use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::spawn_generation::SpawnGeneration;
use crate::{
    adapter::holder::client,
    domain::{
        pty::PtyError,
        value::{PaneId, ProjectRoot},
    },
};

/// The screen of a holder-run process, still to be read from its holder. The
/// read can block, so the supervisor hands it to its caller instead of reading
/// it on its own loop.
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
    /// The socket of the holder that serves it.
    #[getset(get = "pub")]
    socket: PathBuf,
    /// The service's own mirror of the screen, used when the holder cannot be
    /// read.
    #[getset(get = "pub")]
    fallback: String,
}

impl HolderScreen {
    /// Reads the visible screen from the holder, blocking for at most the
    /// holder client's read timeout.
    ///
    /// # Errors
    /// Returns a [`PtyError`] when the holder cannot be reached or sends no
    /// screen.
    pub fn read(&self) -> Result<String, PtyError> {
        client::screen_text(&self.socket)
    }
}
