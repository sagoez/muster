//! The user's global git excludes file, where Claude Code keeps every
//! project's `.claude/settings.local.json` out of git with one rule; muster
//! adds the same rule when it makes that file.

use std::{
    env,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use getset::Getters;
use muster_os::path::home_dir;
use typed_builder::TypedBuilder;

use crate::cli::mcp::{
    agent_file::{read_optional, write_through},
    hook_file_error::HookFileError,
};

/// The rule Claude Code keeps local settings out of git with.
pub const LOCAL_SETTINGS_RULE: &str = "**/.claude/settings.local.json";
/// The variable that moves the user's config folder off `~/.config`.
const CONFIG_HOME_ENV: &str = "XDG_CONFIG_HOME";
/// The user's config folder, in the home folder.
const CONFIG_DIR: &str = ".config";
/// git's folder in the user's config folder.
const GIT_DIR: &str = "git";
/// git's global excludes file in that folder.
const IGNORE_FILE: &str = "ignore";
/// The git binary, asked where the user moved the excludes file.
const GIT: &str = "git";
/// Asks git for the user's own setting, its `~` expanded.
const GLOBAL_PATH_QUERY: [&str; 4] = ["config", "--global", "--get", "--path"];
/// The setting naming the excludes file, when moved.
const EXCLUDES_KEY: &str = "core.excludesFile";
/// Ends each line of the file.
const LINE_END: char = '\n';

/// The global excludes file git reads for this user.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct GitExcludes {
    /// The file: `core.excludesFile` when set, else `git/ignore` in
    /// `$XDG_CONFIG_HOME` or `~/.config`.
    path: PathBuf,
}

impl GitExcludes {
    /// The file git reads, as git itself finds it; none when there is no
    /// home folder to find it in.
    pub fn find() -> Option<Self> {
        let path = configured().or_else(|| {
            env::var_os(CONFIG_HOME_ENV)
                .map(PathBuf::from)
                .filter(|dir| dir.is_absolute())
                .or_else(|| home_dir().map(|home| home.join(CONFIG_DIR)))
                .map(|dir| dir.join(GIT_DIR).join(IGNORE_FILE))
        })?;
        Some(Self::builder().path(path).build())
    }

    /// Adds [`LOCAL_SETTINGS_RULE`] on a line of its own unless a line
    /// already holds it. Returns whether it wrote.
    ///
    /// # Errors
    /// Returns [`HookFileError`] when the file cannot be read or written.
    pub fn keep_local_settings_out(&self) -> Result<bool, HookFileError> {
        let text = read_optional(&self.path)?.unwrap_or_default();
        if text.lines().any(|line| line.trim() == LOCAL_SETTINGS_RULE) {
            return Ok(false);
        }
        let mut kept = text;
        if !kept.is_empty() && !kept.ends_with(LINE_END) {
            kept.push(LINE_END);
        }
        kept.push_str(LOCAL_SETTINGS_RULE);
        kept.push(LINE_END);
        write_through(&self.path, &kept)?;
        Ok(true)
    }
}

/// The excludes file the user named in git's own config, when git runs
/// and the setting names an absolute path.
fn configured() -> Option<PathBuf> {
    let output = Command::new(GIT)
        .args(GLOBAL_PATH_QUERY)
        .arg(EXCLUDES_KEY)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    let named = String::from_utf8(output.stdout).ok()?;
    let path = Path::new(named.trim_end_matches(LINE_END));
    path.is_absolute().then(|| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    /// The rule goes on a line of its own after what the file holds, and
    /// adding it again writes nothing.
    #[test]
    fn the_rule_is_added_once() {
        let scratch = TempDir::new().unwrap();
        let excludes = GitExcludes::builder()
            .path(scratch.path().join("git").join("ignore"))
            .build();

        assert!(excludes.keep_local_settings_out().unwrap());
        assert_eq!(
            fs::read_to_string(excludes.path()).unwrap(),
            format!("{LOCAL_SETTINGS_RULE}\n")
        );

        fs::write(excludes.path(), "*.swp").unwrap();
        assert!(excludes.keep_local_settings_out().unwrap());
        assert!(!excludes.keep_local_settings_out().unwrap());
        assert_eq!(
            fs::read_to_string(excludes.path()).unwrap(),
            format!("*.swp\n{LOCAL_SETTINGS_RULE}\n")
        );
    }
}
