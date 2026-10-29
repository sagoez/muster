use crate::domain::{
    config::{ConfigError, SettingChange},
    settings::Settings,
};

/// Driven port: the machine-wide [`Settings`].
pub trait SettingsStore {
    /// The current settings.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be read or holds an
    /// invalid value.
    fn load(&self) -> Result<Settings, ConfigError>;

    /// Applies `change` and returns the settings it produced.
    ///
    /// # Errors
    /// Returns what [`Settings::with_change`] refuses, or another
    /// [`ConfigError`] when the store cannot be written.
    fn apply(&self, change: &SettingChange) -> Result<Settings, ConfigError>;
}
