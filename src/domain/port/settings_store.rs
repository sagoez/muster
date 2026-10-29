use crate::domain::{
    config::{ConfigError, SettingChange},
    coordination::Author,
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

    /// Applies `change` for `author` and returns the settings it produced.
    /// An agent's change is kept in every project's edit log in the same
    /// write, since every project's new processes take it.
    ///
    /// # Errors
    /// Returns what [`Settings::with_change`] refuses, or another
    /// [`ConfigError`] when the store cannot be written.
    fn apply(&self, change: &SettingChange, author: &Author) -> Result<Settings, ConfigError>;
}
