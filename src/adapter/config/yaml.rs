use std::{
    fs,
    path::{Path, PathBuf},
};

use directories::ProjectDirs;
use getset::Getters;
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::domain::{
    config::{ConfigError, WorkspaceConfig},
    port::ConfigSource,
};

/// Application directory used to locate the platform config directory.
const APP_DIR: &str = "muster";

/// Path to `filename` inside muster's config directory, when one can be resolved
/// (`~/.config/muster/<filename>` on Linux). Shared by the project registry and
/// the settings store.
pub(crate) fn config_dir_path(filename: &str) -> Option<PathBuf> {
    ProjectDirs::from("", "", APP_DIR).map(|dirs| dirs.config_dir().join(filename))
}

/// Reads and parses a `muster.yml`-style workspace config from `path`. Shared by
/// the single-file config source and the project registry.
///
/// # Errors
/// Returns a `ConfigError` if the file cannot be read or is not valid config.
pub(crate) fn load_workspace(path: &Path) -> Result<WorkspaceConfig, ConfigError> {
    let raw = fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(serde_yaml_ng::from_str(&raw)?)
}

/// Serializes `value` to YAML and writes it to `path`, creating any missing
/// parent directories first. The write is atomic: it lands in a sibling
/// temporary file that is then renamed over the destination, so a crash, full
/// disk, or short write can never truncate an existing valid file.
///
/// An existing symlink is followed so its target is rewritten rather than
/// replaced with a regular file. A parentless relative path (e.g. `muster.yml`)
/// writes into the current directory.
///
/// # Errors
/// Returns a `ConfigError::Write` if a directory, temp file, or rename fails.
pub(crate) fn write_config<T: Serialize>(path: &Path, value: &T) -> Result<(), ConfigError> {
    let dest = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if let Some(parent) = dest.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let raw = serde_yaml_ng::to_string(value)?;
    let temp = temp_path(&dest);
    fs::write(&temp, raw).map_err(|source| ConfigError::Write {
        path: temp.clone(),
        source,
    })?;
    fs::rename(&temp, &dest).map_err(|source| {
        let _ = fs::remove_file(&temp);
        ConfigError::Write {
            path: dest.clone(),
            source,
        }
    })
}

/// A sibling temporary path in the same directory as `path`, so the later rename
/// stays on one filesystem and is therefore atomic.
fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.tmp", std::process::id()));
    path.with_file_name(name)
}

/// A [`ConfigSource`] that loads a `muster.yml`-style file from disk.
#[derive(Clone, Debug, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct YamlConfigSource {
    /// Path to the YAML config file.
    path: PathBuf,
}

impl ConfigSource for YamlConfigSource {
    fn load(&self) -> Result<WorkspaceConfig, ConfigError> {
        load_workspace(&self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example_config() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/muster.yml")
    }

    #[test]
    fn loads_the_example_config() {
        let source = YamlConfigSource::builder().path(example_config()).build();
        let config = source.load().unwrap();
        assert!(!config.to_processes().is_empty());
    }

    #[test]
    fn missing_file_is_a_read_error() {
        let source = YamlConfigSource::builder()
            .path(PathBuf::from("/nonexistent/muster.yml"))
            .build();
        assert!(matches!(source.load(), Err(ConfigError::Read { .. })));
    }
}
