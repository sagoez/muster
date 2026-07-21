use crate::domain::{config::ConfigError, settings::Settings};

/// Loads and persists cross-workspace [`Settings`], typically a settings file in
/// the user's config directory.
pub trait SettingsStore {
    /// Loads the current settings, materializing defaults on first run.
    ///
    /// # Errors
    /// Returns a `ConfigError` if an existing settings file cannot be read or
    /// parsed, or if a fresh default file cannot be written.
    fn load(&self) -> Result<Settings, ConfigError>;

    /// Persists `settings`.
    ///
    /// # Errors
    /// Returns a `ConfigError` if the settings file cannot be written.
    fn save(&self, settings: &Settings) -> Result<(), ConfigError>;
}
