use std::{
    ffi::OsStr,
    fs,
    path::{MAIN_SEPARATOR, Path, PathBuf, is_separator},
};

use directories::BaseDirs;

use crate::domain::{config::ConfigError, port::PathCompleter, project::Project};

/// Leading path component expanded to the user's home directory.
const HOME_PREFIX: &str = "~";
/// Maximum directory suggestions offered at once.
const MAX_COMPLETIONS: usize = 8;

/// A [`PathCompleter`] that lists directories on the local filesystem.
#[derive(Default)]
pub struct FsPathCompleter;

impl FsPathCompleter {
    /// Returns directory completions for `partial` from the local filesystem.
    fn complete(partial: &str) -> Vec<String> {
        // When `partial` is itself an existing directory (and not already ending
        // in a separator), browse inside it; otherwise complete the last segment
        // within its parent. `is_separator` accepts both `/` and the platform
        // separator, so Windows-style paths split correctly too.
        let (typed_dir, prefix): (String, String) =
            if !partial.ends_with(is_separator) && expand_home(Path::new(partial)).is_dir() {
                (format!("{partial}{MAIN_SEPARATOR}"), String::new())
            } else {
                match partial.rfind(is_separator) {
                    Some(index) => (
                        partial[..=index].to_string(),
                        partial[index + 1..].to_string(),
                    ),
                    None => (String::new(), partial.to_string()),
                }
            };
        let read_dir = if typed_dir.is_empty() {
            PathBuf::from(".")
        } else {
            expand_home(Path::new(&typed_dir))
        };
        let Ok(entries) = fs::read_dir(&read_dir) else {
            return Vec::new();
        };
        let mut matches: Vec<String> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_dir())
            .filter_map(|entry| Self::candidate(&entry.file_name(), &typed_dir, &prefix))
            .collect();
        matches.sort();
        matches.truncate(MAX_COMPLETIONS);
        matches
    }

    /// Converts one directory name to a completion, rejecting non-UTF-8 names
    /// because a lossy string would identify a different filesystem path.
    fn candidate(raw: &OsStr, typed_dir: &str, prefix: &str) -> Option<String> {
        let name = raw.to_str()?;
        let hidden = name.starts_with('.') && !prefix.starts_with('.');
        (name.starts_with(prefix) && name != prefix && !hidden)
            .then(|| format!("{typed_dir}{name}"))
    }
}

impl PathCompleter for FsPathCompleter {
    fn complete_dir(&self, partial: &str) -> Vec<String> {
        Self::complete(partial)
    }
}

/// Expands a leading `~` to the user's home directory; any other path is left
/// unchanged.
pub fn expand_home(path: &Path) -> PathBuf {
    match path.strip_prefix(HOME_PREFIX) {
        Ok(tail) => match BaseDirs::new() {
            Some(dirs) => dirs.home_dir().join(tail),
            None => path.to_path_buf(),
        },
        Err(_) => path.to_path_buf(),
    }
}

/// Expands `~` and makes a path absolute without resolving symlinks, preserving
/// the user-selected filesystem location.
pub fn absolutize(path: &Path) -> PathBuf {
    let expanded = expand_home(path);
    if expanded.is_absolute() {
        expanded
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(&expanded),
            Err(_) => expanded,
        }
    }
}

/// Resolves a registered config only when its stored path is independent of the
/// caller's directory.
///
/// # Errors
/// Returns [`ConfigError::RelativeProjectConfig`] for ambiguous legacy entries.
pub fn registered_config_path(project: &Project) -> Result<PathBuf, ConfigError> {
    if expand_home(project.config()).is_relative() {
        return Err(ConfigError::RelativeProjectConfig {
            name: project.name().clone(),
            path: project.config().clone(),
        });
    }
    Ok(absolutize(project.config()))
}

/// Normalizes a config path for identity comparison: absolutizes it, then
/// canonicalizes existing paths so aliases naming the same file compare equal.
pub fn normalize(path: &Path) -> PathBuf {
    let absolute = absolutize(path);
    absolute.canonicalize().unwrap_or(absolute)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_home_rewrites_a_tilde_prefix() {
        let home = BaseDirs::new().unwrap().home_dir().to_path_buf();
        assert_eq!(
            expand_home(Path::new("~/Projects/muster.yml")),
            home.join("Projects/muster.yml")
        );
        assert_eq!(
            expand_home(Path::new("/etc/muster.yml")),
            PathBuf::from("/etc/muster.yml")
        );
    }

    #[test]
    fn relative_and_absolute_paths_to_one_file_normalize_equal() {
        // `cargo test` runs with the crate root as the working directory, so the
        // relative name and its absolute form resolve to the same file.
        let absolute = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        assert_eq!(normalize(Path::new("Cargo.toml")), normalize(&absolute));
    }

    #[cfg(unix)]
    /// An absolute location retains a symlink alias while identity normalization
    /// resolves it to its target.
    #[test]
    fn absolutize_preserves_a_symlink_path() {
        use std::os::unix::fs::symlink;

        let dir = std::env::temp_dir().join(format!("muster-path-link-{}", std::process::id()));
        let target = dir.join("target.yml");
        let link = dir.join("muster.yml");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&target, "").unwrap();
        symlink(&target, &link).unwrap();

        assert_eq!(absolutize(&link), link);
        assert_eq!(normalize(&link), target);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn complete_dir_suggests_matching_subdirectories() {
        let dir = std::env::temp_dir().join(format!("muster-complete-{}", std::process::id()));
        for sub in ["prism", "proto", "other"] {
            fs::create_dir_all(dir.join(sub)).unwrap();
        }
        fs::write(dir.join("prfile"), "").unwrap();

        let base = dir.display();
        let got = FsPathCompleter::complete(&format!("{base}/pr"));

        assert_eq!(
            got,
            vec![format!("{base}/prism"), format!("{base}/proto")],
            "only matching directories, prefixed as typed, sorted"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn complete_dir_browses_inside_an_existing_directory() {
        let dir = std::env::temp_dir().join(format!("muster-browse-{}", std::process::id()));
        for sub in ["alpha", "beta"] {
            fs::create_dir_all(dir.join(sub)).unwrap();
        }

        // The path is an existing directory with no trailing slash: list its
        // children, not its siblings.
        let base = dir.display();
        let got = FsPathCompleter::complete(&base.to_string());

        assert_eq!(got, vec![format!("{base}/alpha"), format!("{base}/beta")]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    /// Non-UTF-8 names are rejected without asking the filesystem to create one.
    #[test]
    fn candidate_rejects_non_utf8_names() {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

        let raw = OsStr::from_bytes(b"val\xffid");

        assert_eq!(FsPathCompleter::candidate(raw, "/tmp/", "val"), None);
    }
}
