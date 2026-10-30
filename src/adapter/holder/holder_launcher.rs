use super::config::HolderConfig;
use crate::domain::pty::PtyError;

/// Starts a holder for one child.
pub trait HolderLauncher: Send + Sync {
    /// Starts a holder running `config`, without waiting for it to serve.
    ///
    /// # Errors
    /// Returns a [`PtyError`] when the holder cannot be started.
    fn launch(&self, config: HolderConfig) -> Result<(), PtyError>;
}
