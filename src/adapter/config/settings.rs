use std::{fs, path::PathBuf};

use super::yaml::{config_dir_path, write_config};
use crate::domain::{config::ConfigError, port::SettingsStore, settings::Settings};

/// Settings file name under the muster config directory.
const SETTINGS_FILE: &str = "settings.yml";
/// Shipped first-run settings. Defaults stay explicit in configuration rather
/// than being duplicated as Rust values.
const DEFAULT_SETTINGS: &str = include_str!("default_settings.yml");

/// A [`SettingsStore`] backed by `settings.yml` in the user's config directory
/// (`~/.config/muster/settings.yml` on Linux). On first run it writes the
/// defaults so every value stays explicit on disk rather than living in Rust.
#[derive(Default)]
pub struct YamlSettingsStore;

impl YamlSettingsStore {
    /// Path to the settings file, when a config directory can be resolved.
    fn path() -> Option<PathBuf> {
        config_dir_path(SETTINGS_FILE)
    }

    /// Parses the shipped settings that are materialized on first run.
    fn defaults() -> Result<Settings, ConfigError> {
        Ok(serde_yaml_ng::from_str(DEFAULT_SETTINGS)?)
    }

    /// Loads settings from `path`, failing closed when no platform config
    /// directory exists so an enabled default never becomes impossible to save.
    fn load_from(path: Option<PathBuf>) -> Result<Settings, ConfigError> {
        let path = path.ok_or(ConfigError::NoConfigDir)?;
        if !path.exists() {
            let defaults = Self::defaults()?;
            write_config(&path, &defaults)?;
            return Ok(defaults);
        }
        let raw = fs::read_to_string(&path).map_err(|source| ConfigError::Read {
            path: path.clone(),
            source,
        })?;
        Ok(serde_yaml_ng::from_str(&raw)?)
    }
}

impl SettingsStore for YamlSettingsStore {
    fn load(&self) -> Result<Settings, ConfigError> {
        Self::load_from(Self::path())
    }

    fn save(&self, settings: &Settings) -> Result<(), ConfigError> {
        let path = Self::path().ok_or(ConfigError::NoConfigDir)?;
        write_config(&path, settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_config_directory_fails_closed() {
        assert!(matches!(
            YamlSettingsStore::load_from(None),
            Err(ConfigError::NoConfigDir)
        ));
    }

    #[test]
    fn shipped_defaults_are_explicit_and_valid() {
        let settings = YamlSettingsStore::defaults().unwrap();

        assert!(*settings.desktop_notifications());
    }
}
