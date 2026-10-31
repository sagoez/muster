use std::{
    ffi::OsString,
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

use getset::Getters;
use typed_builder::TypedBuilder;

#[cfg(not(target_os = "macos"))]
use crate::constants::XDG_CONFIG_HOME_ENV;
use crate::{
    adapter::client::first_on_path,
    constants::{DOT_CONFIG, PATH_ENV},
};

/// The variable that moves Claude Code's config folder away from home.
pub const CLAUDE_CONFIG_DIR_ENV: &str = "CLAUDE_CONFIG_DIR";
/// The variable that moves Codex's home folder.
pub const CODEX_HOME_ENV: &str = "CODEX_HOME";
/// Claude Code's own file of settings, user servers and local servers.
const CLAUDE_JSON: &str = ".claude.json";
/// The older name of Claude Code's file, in its config folder, which it
/// still reads in place of `.claude.json` when it holds anything.
const CLAUDE_LEGACY_JSON: &str = ".config.json";
/// The write bits of a file's mode, for its owner, group and others.
const ANY_WRITE: u32 = 0o222;
/// Claude Code's folder under home, present once it has run.
const CLAUDE_DIR: &str = ".claude";
/// Codex's home folder under the user's home.
const CODEX_DIR: &str = ".codex";
/// Zed's folder in the config root.
const ZED_DIR: &str = "zed";
/// Zed's user settings file.
const ZED_SETTINGS: &str = "settings.json";

/// Where each agent keeps the configs muster edits for the user, and the
/// folders their binaries are looked up in. Built from the environment by
/// the composition root, or from a scratch folder by a test.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
pub struct ConfigLocations {
    /// Claude Code's `.claude.json`, which holds its user and local servers.
    #[getset(get = "pub")]
    claude_json: PathBuf,
    /// Claude Code's config folder, whose presence says it is installed.
    #[getset(get = "pub")]
    claude_dir: PathBuf,
    /// Codex's home folder, holding `config.toml`.
    #[getset(get = "pub")]
    codex_home: PathBuf,
    /// Zed's user settings file.
    #[getset(get = "pub")]
    zed_settings: PathBuf,
    /// The `PATH` value binaries are looked up in, if any.
    path: Option<OsString>,
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
        let claude_config = absolute(CLAUDE_CONFIG_DIR_ENV);
        Self::builder()
            .claude_json(claude_config.as_deref().unwrap_or(home).join(CLAUDE_JSON))
            .claude_dir(claude_config.unwrap_or_else(|| home.join(CLAUDE_DIR)))
            .codex_home(absolute(CODEX_HOME_ENV).unwrap_or_else(|| home.join(CODEX_DIR)))
            .zed_settings(zed_settings(home, &absolute))
            .path(var(PATH_ENV))
            .build()
    }

    /// The file Claude Code keeps its user and local servers in: its older
    /// `.config.json` in its config folder when that is a file holding
    /// anything, or an empty but writable one with no `.claude.json` beside
    /// it, as Claude Code itself chooses; else `.claude.json`.
    pub fn claude_config(&self) -> PathBuf {
        let legacy = self.claude_dir.join(CLAUDE_LEGACY_JSON);
        let chosen = fs::metadata(&legacy).is_ok_and(|meta| {
            meta.is_file()
                && (meta.len() > 0 || (meta.mode() & ANY_WRITE != 0 && !self.claude_json.exists()))
        });
        if chosen {
            legacy
        } else {
            self.claude_json.clone()
        }
    }

    /// Whether one of `names` is a runnable file in an absolute `PATH`
    /// folder.
    pub fn on_path(&self, names: &[&str]) -> bool {
        self.path
            .as_deref()
            .is_some_and(|path| names.iter().any(|name| first_on_path(path, name).is_some()))
    }
}

/// Zed's settings file under `home`: in the config root, on Linux the
/// absolute `XDG_CONFIG_HOME`, else `~/.config`.
#[cfg(not(target_os = "macos"))]
fn zed_settings(home: &Path, absolute: &impl Fn(&str) -> Option<PathBuf>) -> PathBuf {
    absolute(XDG_CONFIG_HOME_ENV)
        .unwrap_or_else(|| home.join(DOT_CONFIG))
        .join(ZED_DIR)
        .join(ZED_SETTINGS)
}

/// Zed's settings file under `home`: on macOS always in `~/.config`.
#[cfg(target_os = "macos")]
fn zed_settings(home: &Path, _absolute: &impl Fn(&str) -> Option<PathBuf>) -> PathBuf {
    home.join(DOT_CONFIG).join(ZED_DIR).join(ZED_SETTINGS)
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt};

    use tempfile::TempDir;

    use super::*;
    use crate::constants::XDG_CONFIG_HOME_ENV;

    /// A mode anyone may execute.
    const EXECUTABLE_MODE: u32 = 0o755;

    /// Each agent's home-relative default, and the variables that move them.
    #[test]
    fn locations_follow_each_agents_variables() {
        let home = Path::new("/h");
        let plain = ConfigLocations::from_env(home, |_| None);
        assert_eq!(plain.claude_json(), Path::new("/h/.claude.json"));
        assert_eq!(plain.claude_dir(), Path::new("/h/.claude"));
        assert_eq!(plain.codex_home(), Path::new("/h/.codex"));
        assert_eq!(
            plain.zed_settings(),
            Path::new("/h/.config/zed/settings.json")
        );

        let moved = ConfigLocations::from_env(home, |name| match name {
            CLAUDE_CONFIG_DIR_ENV => Some("/c".into()),
            CODEX_HOME_ENV => Some("/x".into()),
            XDG_CONFIG_HOME_ENV => Some("/cfg".into()),
            _ => None,
        });
        assert_eq!(moved.claude_json(), Path::new("/c/.claude.json"));
        assert_eq!(
            moved.claude_dir(),
            Path::new("/c"),
            "Claude Code's folder moves with its config"
        );
        assert_eq!(moved.codex_home(), Path::new("/x"));
        let zed = if cfg!(target_os = "macos") {
            "/h/.config/zed/settings.json"
        } else {
            "/cfg/zed/settings.json"
        };
        assert_eq!(moved.zed_settings(), Path::new(zed));

        let relative = ConfigLocations::from_env(home, |_| Some("rel".into()));
        assert_eq!(
            relative.codex_home(),
            Path::new("/h/.codex"),
            "a relative override is ignored"
        );
    }

    /// Claude Code's older `.config.json` is its file when it holds
    /// anything, or is empty and writable with no `.claude.json` beside it;
    /// otherwise `.claude.json` is.
    #[test]
    fn claude_codes_older_file_wins_as_claude_code_chooses() {
        let scratch = TempDir::new().unwrap();
        let home = scratch.path();
        let locations = ConfigLocations::from_env(home, |_| None);
        let legacy = home.join(".claude/.config.json");
        let current = home.join(".claude.json");
        assert_eq!(locations.claude_config(), current, "no older file");

        fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        fs::write(&legacy, "").unwrap();
        assert_eq!(locations.claude_config(), legacy, "empty, writable, alone");
        fs::write(&current, "{}").unwrap();
        assert_eq!(locations.claude_config(), current, "empty beside the file");
        fs::write(&legacy, "{}").unwrap();
        assert_eq!(locations.claude_config(), legacy, "holding something");

        fs::remove_file(&legacy).unwrap();
        fs::create_dir(&legacy).unwrap();
        assert_eq!(locations.claude_config(), current, "a folder is no file");
    }

    /// A binary counts as on `PATH` only as an executable file in an
    /// absolute folder.
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
        assert!(!ConfigLocations::from_env(Path::new("/h"), |_| None).on_path(&["codex"]));
    }
}
