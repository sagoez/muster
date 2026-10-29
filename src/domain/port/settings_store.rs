use super::SettingsReader;
use crate::domain::{
    config::{AppliedSettings, ConfigError, SettingChange},
    coordination::Author,
};

/// Driven port: the machine-wide [`Settings`](crate::domain::settings::Settings),
/// read and changed.
pub trait SettingsStore: SettingsReader {
    /// Applies `change` for `author` and returns the settings it left, saying
    /// whether they changed. A change that alters the stored settings is kept
    /// in every project's edit log in the same write, since every project's
    /// new processes take it; one to the value already stored is not.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be read or written.
    fn apply(
        &self,
        change: &SettingChange,
        author: &Author,
    ) -> Result<AppliedSettings, ConfigError>;
}
