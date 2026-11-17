//! The exclude file of the git repository a folder is in, where muster keeps
//! the `.claude/settings.local.json` it makes out of git, with the rule
//! Claude Code uses, in that repository alone.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::cli::mcp::{
    agent_file::{read_optional, write_through},
    hook_file_error::HookFileError,
};

/// The rule Claude Code keeps local settings out of git with.
pub const LOCAL_SETTINGS_RULE: &str = "**/.claude/settings.local.json";
/// The git binary, asked where the repository keeps its exclude file.
const GIT: &str = "git";
/// Asks git for the path of a file in the repository's git folder, right
/// for a worktree too.
const GIT_PATH_QUERY: [&str; 2] = ["rev-parse", "--git-path"];
/// The repository's own exclude file, inside its git folder.
const EXCLUDE_FILE: &str = "info/exclude";
/// Ends each line of the file.
const LINE_END: char = '\n';

/// One repository's exclude file.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct GitExcludes {
    /// The file, as git names it.
    path: PathBuf,
}

impl GitExcludes {
    /// The exclude file of the repository holding `folder`, as git finds
    /// it; none when git cannot run or `folder` is in no repository.
    pub fn of(folder: &Path) -> Option<Self> {
        let output = Command::new(GIT)
            .args(GIT_PATH_QUERY)
            .arg(EXCLUDE_FILE)
            .current_dir(folder)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()
            .filter(|output| output.status.success())?;
        let named = String::from_utf8(output.stdout).ok()?;
        let named = named.trim_end_matches(LINE_END);
        if named.is_empty() {
            return None;
        }
        Some(Self::builder().path(folder.join(named)).build())
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

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    /// A fresh repository in a scratch folder, made by git itself.
    fn repository() -> TempDir {
        let scratch = TempDir::new().unwrap();
        let made = Command::new(GIT)
            .args(["init", "-q"])
            .current_dir(scratch.path())
            .status()
            .unwrap();
        assert!(made.success());
        scratch
    }

    /// From a folder inside a repository, the rule goes in that
    /// repository's own exclude file, on a line of its own after what it
    /// holds, and adding it again writes nothing.
    #[test]
    fn the_rule_goes_in_the_repositorys_exclude_file_once() {
        let repository = repository();
        let inner = repository.path().join("app");
        fs::create_dir_all(&inner).unwrap();

        let excludes = GitExcludes::of(&inner).unwrap();
        assert_eq!(
            excludes.path().canonicalize().unwrap(),
            repository
                .path()
                .join(".git")
                .join("info")
                .join("exclude")
                .canonicalize()
                .unwrap()
        );
        fs::write(excludes.path(), "*.swp").unwrap();
        assert!(excludes.keep_local_settings_out().unwrap());
        assert!(!excludes.keep_local_settings_out().unwrap());
        assert_eq!(
            fs::read_to_string(excludes.path()).unwrap(),
            format!("*.swp\n{LOCAL_SETTINGS_RULE}\n")
        );
    }

    /// A folder in no repository has no exclude file.
    #[test]
    fn a_folder_in_no_repository_has_none() {
        let scratch = TempDir::new().unwrap();

        assert_eq!(GitExcludes::of(scratch.path()), None);
    }
}
