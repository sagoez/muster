use std::path::Path;

use thiserror::Error;

use crate::domain::config::ConfigError;

/// Opens a project directory in the user's editor. A driven port so the domain
/// never names a concrete editor or touches the process API. The editor is
/// launched as its own detached window, so it carries no thread-safety bound and
/// does not block the runtime loop.
pub trait EditorLauncher {
    /// Launches the user's editor on `directory` as a detached process in its own
    /// window and returns at once, leaving muster's terminal untouched.
    ///
    /// # Errors
    /// Returns an [`EditorError`] when no editor is configured or the launch fails
    /// to start. A detached editor is not waited on, so its own exit is not
    /// reported.
    fn open(&self, directory: &Path) -> Result<(), EditorError>;
}

/// Why an editor launch could not start.
#[derive(Debug, Error)]
pub enum EditorError {
    /// Neither `$VISUAL` nor `$EDITOR` is set, so no editor is configured.
    #[error("no editor configured (set $VISUAL or $EDITOR)")]
    NoEditor,
    /// The editor command was found but could not be started.
    #[error("could not launch {editor}: {source}")]
    Spawn {
        /// The editor command that failed to start.
        editor: String,
        /// The underlying spawn error.
        source: std::io::Error,
    },
    /// A terminal editor needs a terminal window but this platform has no default
    /// launcher, so it would have no usable tty. The user must set `editor.terminal`.
    #[error(
        "terminal editor {editor} needs a terminal window; set editor.terminal in settings.yml"
    )]
    NoTerminalLauncher {
        /// The terminal editor that has no window launcher.
        editor: String,
    },
    /// The editor settings could not be loaded, so the configured editor is
    /// unknown. Surfaced rather than silently falling back to a different editor.
    #[error("could not load editor settings: {source}")]
    Settings {
        /// The underlying settings load failure.
        #[from]
        source: ConfigError,
    },
}
