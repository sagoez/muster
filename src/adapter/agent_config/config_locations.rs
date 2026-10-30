use std::{
    env,
    ffi::OsString,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use getset::Getters;
use typed_builder::TypedBuilder;

/// The variable that moves Claude Code's config folder away from home.
const CLAUDE_CONFIG_DIR_ENV: &str = "CLAUDE_CONFIG_DIR";
/// The variable that moves Codex's home folder.
const CODEX_HOME_ENV: &str = "CODEX_HOME";
/// The variable naming the XDG config root, which Zed reads on Linux.
#[cfg(not(target_os = "macos"))]
const XDG_CONFIG_HOME_ENV: &str = "XDG_CONFIG_HOME";
/// The variable listing the folders binaries are looked up in.
const PATH_ENV: &str = "PATH";
/// Claude Code's own file of settings, user servers and local servers.
const CLAUDE_JSON: &str = ".claude.json";
/// Claude Code's folder under home, present once it has run.
const CLAUDE_DIR: &str = ".claude";
/// Codex's home folder under the user's home.
const CODEX_DIR: &str = ".codex";
/// The conventional config root under home.
const DOT_CONFIG: &str = ".config";
/// Zed's folder in the config root.
const ZED_DIR: &str = "zed";
/// Zed's user settings file.
const ZED_SETTINGS: &str = "settings.json";
/// Any execute bit of a file's mode.
const EXECUTE_BITS: u32 = 0o111;

/// Where each agent keeps the configs muster edits for the user, and the
/// folders their binaries are looked up in. Built from the environment by
/// the composition root, or from a scratch folder by a test.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ConfigLocations {
    /// Claude Code's `.claude.json`, which holds its user and local servers.
    claude_json: PathBuf,
    /// Claude Code's folder, whose presence says it is installed.
    claude_dir: PathBuf,
    /// Codex's home folder, holding `config.toml`.
    codex_home: PathBuf,
    /// Zed's user `settings.json`.
    zed_settings: PathBuf,
    /// The folders of `PATH`, in order.
    #[builder(default)]
    path: Vec<PathBuf>,
}

impl ConfigLocations {
    /// The locations for a user whose home is `home`, as each agent finds
    /// its own from the variables `var` reads.
    pub fn from_env(home: &Path, var: impl Fn(&str) -> Option<OsString>) -> Self {
        let absolute = |name: &str| {
            var(name)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
        };
        let claude_json = absolute(CLAUDE_CONFIG_DIR_ENV)
            .unwrap_or_else(|| home.to_path_buf())
            .join(CLAUDE_JSON);
        Self::builder()
            .claude_json(claude_json)
            .claude_dir(home.join(CLAUDE_DIR))
            .codex_home(absolute(CODEX_HOME_ENV).unwrap_or_else(|| home.join(CODEX_DIR)))
            .zed_settings(
                zed_config_root(home, &absolute)
                    .join(ZED_DIR)
                    .join(ZED_SETTINGS),
            )
            .path(
                var(PATH_ENV)
                    .map(|path| env::split_paths(&path).collect())
                    .unwrap_or_default(),
            )
            .build()
    }

    /// Whether one of `names` is an executable file in a `PATH` folder.
    pub fn on_path(&self, names: &[&str]) -> bool {
        self.path
            .iter()
            .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
            .any(|candidate| is_executable(&candidate))
    }
}

/// The config root Zed reads its folder from: on Linux the absolute
/// `XDG_CONFIG_HOME`, else `~/.config`.
#[cfg(not(target_os = "macos"))]
fn zed_config_root(home: &Path, absolute: &impl Fn(&str) -> Option<PathBuf>) -> PathBuf {
    absolute(XDG_CONFIG_HOME_ENV).unwrap_or_else(|| home.join(DOT_CONFIG))
}

/// The config root Zed reads its folder from: on macOS always `~/.config`.
#[cfg(target_os = "macos")]
fn zed_config_root(home: &Path, _absolute: &impl Fn(&str) -> Option<PathBuf>) -> PathBuf {
    home.join(DOT_CONFIG)
}

/// Whether `path` is a file someone may execute.
fn is_executable(path: &Path) -> bool {
    path.metadata()
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & EXECUTE_BITS != 0)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    /// A mode anyone may execute.
    const EXECUTABLE_MODE: u32 = 0o755;

    /// Each agent's home-relative default, and the variables that move them.
    #[test]
    fn locations_follow_each_agents_variables() {
        let home = Path::new("/h");
        let plain = ConfigLocations::from_env(home, |_| None);
        assert_eq!(plain.claude_json(), Path::new("/h/.claude.json"));
        assert_eq!(plain.codex_home(), Path::new("/h/.codex"));
        assert_eq!(
            plain.zed_settings(),
            Path::new("/h/.config/zed/settings.json")
        );

        let moved = ConfigLocations::from_env(home, |name| match name {
            CLAUDE_CONFIG_DIR_ENV => Some("/c".into()),
            CODEX_HOME_ENV => Some("/x".into()),
            "XDG_CONFIG_HOME" => Some("/cfg".into()),
            _ => None,
        });
        assert_eq!(moved.claude_json(), Path::new("/c/.claude.json"));
        assert_eq!(moved.codex_home(), Path::new("/x"));
        if cfg!(not(target_os = "macos")) {
            assert_eq!(moved.zed_settings(), Path::new("/cfg/zed/settings.json"));
        }

        let relative = ConfigLocations::from_env(home, |_| Some("rel".into()));
        assert_eq!(
            relative.codex_home(),
            Path::new("/h/.codex"),
            "a relative override is ignored"
        );
    }

    /// A binary counts as on `PATH` only as an executable file.
    #[test]
    fn a_binary_is_found_only_when_executable() {
        let scratch = TempDir::new().unwrap();
        let bin = scratch.path().join("codex");
        fs::write(&bin, "").unwrap();
        let locations = ConfigLocations::from_env(Path::new("/h"), |name| {
            (name == PATH_ENV).then(|| scratch.path().as_os_str().to_owned())
        });

        assert!(!locations.on_path(&["codex"]));
        fs::set_permissions(&bin, fs::Permissions::from_mode(EXECUTABLE_MODE)).unwrap();
        assert!(locations.on_path(&["codex"]));
        assert!(!locations.on_path(&["claude"]));
    }
}
