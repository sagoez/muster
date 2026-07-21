use std::{
    fs,
    path::{Path, PathBuf},
};

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::{
    config::{ConfigError, WorkspaceConfig},
    port::ConfigSource,
};

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
