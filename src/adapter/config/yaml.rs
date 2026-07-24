use std::{
    fs,
    io::{ErrorKind, Write},
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

/// Path to `filename` inside muster's platform state directory, falling back to
/// local data on platforms without a distinct state location.
pub(crate) fn state_dir_path(filename: &str) -> Option<PathBuf> {
    ProjectDirs::from("", "", APP_DIR).map(|dirs| {
        dirs.state_dir()
            .unwrap_or_else(|| dirs.data_local_dir())
            .join(filename)
    })
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
    let config: WorkspaceConfig = serde_yaml_ng::from_str(&raw)?;
    config.validate()?;
    Ok(config)
}

/// Writes `value` to `path` only when nothing exists there. The complete
/// contents land in an exclusively created, unpredictably named staging file
/// and are published with a no-replace hard link, so a concurrent creation is
/// never clobbered, a dangling symlink is refused, and an interrupted write
/// can never leave a truncated destination blocking a later attempt. Where
/// the filesystem has no hard links, publication falls back to an exclusive
/// direct write. Returns whether the file was created.
///
/// # Errors
/// Returns a `ConfigError` when serialization, the write, or the publish
/// fails.
pub(crate) fn create_config_new<T: Serialize>(path: &Path, value: &T) -> Result<bool, ConfigError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let raw = serde_yaml_ng::to_string(value)?;
    let staging = write_staging(path, &raw)?;
    let published = publish_new(&staging, path, &raw);
    let _ = fs::remove_file(&staging);
    published
}

/// Publishes `staging` at `path` without ever replacing an existing file: a
/// hard link where the filesystem supports one, else an exclusive direct
/// write of `raw`. The fallback removes its partial file when the write
/// fails; only a hard kill mid-write can leave one behind there.
fn publish_new(staging: &Path, path: &Path, raw: &str) -> Result<bool, ConfigError> {
    match fs::hard_link(staging, path) {
        Ok(()) => return Ok(true),
        Err(source) if source.kind() == ErrorKind::AlreadyExists => return Ok(false),
        // Any other failure (e.g. a filesystem without hard links) falls
        // through to the exclusive direct write.
        Err(_) => {},
    }
    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(source) if source.kind() == ErrorKind::AlreadyExists => return Ok(false),
        Err(source) => {
            return Err(ConfigError::Write {
                path: path.to_path_buf(),
                source,
            });
        },
    };
    file.write_all(raw.as_bytes()).map_err(|source| {
        let _ = fs::remove_file(path);
        ConfigError::Write {
            path: path.to_path_buf(),
            source,
        }
    })?;
    Ok(true)
}

/// Creates an exclusive staging file beside `path` and writes `raw` into it.
/// The exclusive open never follows a pre-planted symlink, and the
/// unpredictable name defeats guessing the path in shared directories.
fn write_staging(path: &Path, raw: &str) -> Result<PathBuf, ConfigError> {
    let staging = staging_path(path);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staging)
        .map_err(|source| ConfigError::Write {
            path: staging.clone(),
            source,
        })?;
    file.write_all(raw.as_bytes()).map_err(|source| {
        let _ = fs::remove_file(&staging);
        ConfigError::Write {
            path: staging.clone(),
            source,
        }
    })?;
    Ok(staging)
}

/// An unpredictable sibling staging path for `path`.
fn staging_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}{STAGING_SUFFIX}", uuid::Uuid::new_v4()));
    path.with_file_name(name)
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
    let staging = write_staging(&dest, &raw)?;
    fs::rename(&staging, &dest).map_err(|source| {
        let _ = fs::remove_file(&staging);
        ConfigError::Write {
            path: dest.clone(),
            source,
        }
    })
}

/// Suffix marking a staging file awaiting publication. Staging lives beside
/// its destination so the rename stays on one filesystem and is atomic.
const STAGING_SUFFIX: &str = ".tmp";

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

    /// The exclusive create writes once and never clobbers an existing file.
    #[test]
    fn create_config_new_refuses_to_overwrite() {
        let dir = std::env::temp_dir().join(format!("muster-create-{}", std::process::id()));
        let path = dir.join("muster.yml");
        std::fs::create_dir_all(&dir).unwrap();

        assert!(create_config_new(&path, &"first").unwrap(), "first create");
        assert!(
            !create_config_new(&path, &"second").unwrap(),
            "existing file wins"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), "first");
        let residue = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(residue, 0, "no temporary files survive either outcome");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A dangling symlink at the destination is refused, not followed.
    #[cfg(unix)]
    #[test]
    fn create_config_new_refuses_a_dangling_symlink_destination() {
        use std::os::unix::fs::symlink;

        let dir = std::env::temp_dir().join(format!("muster-create-link-{}", std::process::id()));
        let path = dir.join("muster.yml");
        std::fs::create_dir_all(&dir).unwrap();
        symlink(dir.join("missing.yml"), &path).unwrap();

        assert!(
            !create_config_new(&path, &"content").unwrap(),
            "an occupied path, even a dangling symlink, is never replaced"
        );
        assert!(
            !dir.join("missing.yml").exists(),
            "nothing was written through the symlink"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

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
