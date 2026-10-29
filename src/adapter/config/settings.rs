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
    /// directory exists so an enabled default never becomes impossible to save,
    /// and refusing a `default_size` outside the PTY bounds.
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
        let settings: Settings =
            serde_yaml_ng::from_str(&raw).map_err(|source| ConfigError::ParseFile {
                path: path.clone(),
                source,
            })?;
        settings
            .default_size()
            .checked()
            .map_err(|source| ConfigError::InvalidDefaultSize { path, source })?;
        Ok(settings)
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
    use crate::domain::color_scheme::ColorScheme;

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
        assert_eq!(*settings.color_scheme(), ColorScheme::Dark);
        assert_eq!(settings.default_size().rows().into_inner(), 24);
        assert_eq!(settings.default_size().cols().into_inner(), 80);
        assert_eq!(settings.detach_key().as_ref(), "ctrl-\\");
        assert_eq!(settings.log_max_size().bytes(), 10 * 1024 * 1024);
        assert_eq!(*settings.scrollback(), 1000);
    }

    /// A default size no PTY can take is refused naming the setting, rather
    /// than reaching a holder that cannot render it.
    #[test]
    fn a_default_size_outside_the_pty_bounds_is_refused() {
        let zero_rows = DEFAULT_SETTINGS.replace("rows: 24", "rows: 0");
        assert_ne!(zero_rows, DEFAULT_SETTINGS, "the fixture changed the rows");

        let scratch = tempfile::TempDir::new().unwrap();
        let path = scratch.path().join(SETTINGS_FILE);
        fs::write(&path, zero_rows).unwrap();

        let message = YamlSettingsStore::load_from(Some(path))
            .unwrap_err()
            .to_string();
        assert!(
            message.contains("default_size"),
            "names the setting: {message}"
        );
        assert!(message.contains("0x80"), "names the size: {message}");
    }

    /// Every setting is explicit: a file missing one fails to load rather than
    /// falling back to a value hidden in code.
    #[test]
    fn a_settings_file_missing_a_setting_is_refused() {
        let without_detach: String = DEFAULT_SETTINGS
            .lines()
            .filter(|line| !line.starts_with("detach_key"))
            .collect::<Vec<_>>()
            .join("\n");

        let scratch = tempfile::TempDir::new().unwrap();
        let path = scratch.path().join(SETTINGS_FILE);
        fs::write(&path, without_detach).unwrap();

        let error = YamlSettingsStore::load_from(Some(path.clone())).unwrap_err();

        let message = error.to_string();
        assert!(message.contains("detach_key"), "names the key: {message}");
        assert!(
            message.contains(&path.display().to_string()),
            "names the file: {message}"
        );
    }
}
