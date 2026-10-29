use crate::domain::{config::ConfigError, settings::Settings};

/// Driven port: reads the machine-wide [`Settings`], for a reader that never
/// changes them.
pub trait SettingsReader {
    /// The current settings.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be read or holds an
    /// invalid value.
    fn load(&self) -> Result<Settings, ConfigError>;
}
