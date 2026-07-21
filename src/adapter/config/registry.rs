use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::yaml::{config_dir_path, load_workspace, write_config};
use crate::{
    adapter::path::expand_home,
    domain::{
        config::{ConfigError, WorkspaceConfig},
        port::ProjectRegistry,
        project::Project,
    },
};

/// Registry file name under the muster config directory.
const REGISTRY_FILE: &str = "projects.yml";

/// On-disk registry shape: a top-level `projects:` list.
#[derive(Serialize, Deserialize)]
struct RegistryFile {
    projects: Vec<Project>,
}

/// A [`ProjectRegistry`] backed by `projects.yml` in the user's config directory
/// (`~/.config/muster/projects.yml` on Linux). A project's `config` path may
/// begin with `~`, expanded to the home directory when the workspace is loaded.
#[derive(Default)]
pub struct YamlProjectRegistry;

impl YamlProjectRegistry {
    /// Path to the registry file, when a config directory can be resolved.
    fn registry_path() -> Option<PathBuf> {
        config_dir_path(REGISTRY_FILE)
    }
}

impl ProjectRegistry for YamlProjectRegistry {
    fn projects(&self) -> Result<Vec<Project>, ConfigError> {
        let Some(path) = Self::registry_path() else {
            return Ok(Vec::new());
        };
        if !path.exists() {
            return Ok(Vec::new());
        }
        let raw = fs::read_to_string(&path).map_err(|source| ConfigError::Read {
            path: path.clone(),
            source,
        })?;
        let file: RegistryFile = serde_yaml_ng::from_str(&raw)?;
        Ok(file.projects)
    }

    fn workspace(&self, config_path: &Path) -> Result<WorkspaceConfig, ConfigError> {
        load_workspace(&expand_home(config_path))
    }

    fn workspace_exists(&self, config_path: &Path) -> bool {
        expand_home(config_path).exists()
    }

    fn save(&self, projects: &[Project]) -> Result<(), ConfigError> {
        let path = Self::registry_path().ok_or(ConfigError::NoConfigDir)?;
        let file = RegistryFile {
            projects: projects.to_vec(),
        };
        write_config(&path, &file)
    }

    fn save_workspace(
        &self,
        config_path: &Path,
        config: &WorkspaceConfig,
    ) -> Result<(), ConfigError> {
        write_config(&expand_home(config_path), config)
    }

    fn update_workspace(
        &self,
        config_path: &Path,
        update: &mut dyn FnMut(WorkspaceConfig) -> WorkspaceConfig,
    ) -> Result<(), ConfigError> {
        // Canonicalize first so a symlink and its real path resolve to one lock
        // file; `write_config` canonicalizes the destination the same way, so
        // otherwise the two addressings would take different locks and race.
        let expanded = expand_home(config_path);
        let dest = expanded.canonicalize().unwrap_or(expanded);
        // Hold an exclusive lock across the read-modify-write so two concurrent
        // `muster run` invocations serialize instead of clobbering each other. A
        // lock failure aborts the update rather than risking a silent lost write.
        let _guard = lock_workspace(&dest)?;
        let config = load_workspace(&dest)?;
        write_config(&dest, &update(config))
    }
}

/// Acquires an exclusive advisory lock for the workspace at `dest`, on a stable
/// sibling `.lock` file (never renamed, unlike the config itself). The lock
/// releases when the returned handle is dropped.
///
/// # Errors
/// Returns a `ConfigError` if the lock file cannot be opened or locked, so a
/// caller never proceeds believing it holds a lock it does not.
#[cfg(unix)]
fn lock_workspace(dest: &Path) -> Result<fs::File, ConfigError> {
    use std::os::unix::io::AsRawFd;

    let lock_path = lock_path_of(dest);
    if let Some(parent) = lock_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&lock_path)
        .map_err(|source| ConfigError::Write {
            path: lock_path.clone(),
            source,
        })?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == -1 {
        return Err(ConfigError::Write {
            path: lock_path,
            source: std::io::Error::last_os_error(),
        });
    }
    Ok(file)
}

/// No advisory lock off Unix: the TUI is single-process there and `muster run`
/// is Unix-only, so there is no cross-process writer to serialize against.
#[cfg(not(unix))]
fn lock_workspace(_dest: &Path) -> Result<(), ConfigError> {
    Ok(())
}

/// The sibling `<name>.lock` path for a config file.
fn lock_path_of(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".lock");
    dest.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::domain::{
        config::ProcessSpec,
        process::{StopPolicy, StopSignal},
        value::{CommandLine, ProcessName},
    };

    /// Grace period used to verify workspace serialization.
    const ROUND_TRIP_STOP_GRACE: Duration = Duration::from_secs(5);

    #[test]
    fn registry_file_parses_a_projects_list() {
        let raw = "projects:\n  - name: muster\n    config: ~/Projects/muster/muster.yml\n";
        let file: RegistryFile = serde_yaml_ng::from_str(raw).unwrap();
        assert_eq!(file.projects.len(), 1);
        assert_eq!(file.projects[0].name().as_ref(), "muster");
    }

    #[test]
    fn save_workspace_creates_the_file_and_round_trips_through_load() {
        let dir = std::env::temp_dir().join(format!("muster-save-{}", std::process::id()));
        let path = dir.join("nested").join("muster.yml");
        let config = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![
                ProcessSpec::builder()
                    .name(ProcessName::try_new("Shell").unwrap())
                    .build(),
            ])
            .commands(vec![
                ProcessSpec::builder()
                    .name(ProcessName::try_new("Server").unwrap())
                    .command(Some(CommandLine::try_new("serve").unwrap()))
                    .stop(Some(
                        StopPolicy::builder()
                            .signal(StopSignal::Terminate)
                            .grace_period(ROUND_TRIP_STOP_GRACE)
                            .build(),
                    ))
                    .build(),
            ])
            .build();

        let registry = YamlProjectRegistry;
        registry.save_workspace(&path, &config).unwrap();
        assert!(
            path.exists(),
            "the workspace file and its parents were created"
        );

        let loaded = registry.workspace(&path).unwrap();
        assert_eq!(loaded.terminals()[0].name().as_ref(), "Shell");
        let stop = loaded.commands()[0].stop().as_ref().unwrap();
        assert_eq!(*stop.signal(), StopSignal::Terminate);
        assert_eq!(*stop.grace_period(), ROUND_TRIP_STOP_GRACE);

        let leftover_temps = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp"))
            .count();
        assert_eq!(
            leftover_temps, 0,
            "the atomic write leaves no temp file behind"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_workspace_handles_a_parentless_relative_path() {
        // A bare filename's parent is the empty path; create_dir_all("") would
        // fail, which broke `--config muster.yml`. It must write into the cwd.
        let name = format!("muster-parentless-{}.yml", std::process::id());
        let path = PathBuf::from(&name);
        let config = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![])
            .commands(vec![])
            .build();

        let result = YamlProjectRegistry.save_workspace(&path, &config);
        let existed = path.exists();
        let _ = fs::remove_file(&path);

        result.unwrap();
        assert!(existed, "the config is written into the current directory");
    }

    #[cfg(unix)]
    #[test]
    fn save_workspace_rewrites_a_symlink_target_in_place() {
        use std::os::unix::fs::symlink;

        let dir = std::env::temp_dir().join(format!("muster-symlink-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("real.yml");
        let link = dir.join("muster.yml");
        fs::write(&target, "agents: []\nterminals: []\ncommands: []\n").unwrap();
        symlink(&target, &link).unwrap();

        let config = WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![
                ProcessSpec::builder()
                    .name(ProcessName::try_new("Shell").unwrap())
                    .build(),
            ])
            .commands(vec![])
            .build();
        YamlProjectRegistry.save_workspace(&link, &config).unwrap();

        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "the symlink is preserved, not replaced with a regular file"
        );
        let loaded = YamlProjectRegistry.workspace(&target).unwrap();
        assert_eq!(
            loaded.terminals()[0].name().as_ref(),
            "Shell",
            "the symlink's target received the update"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
