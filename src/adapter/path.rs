use std::{
    ffi::OsStr,
    fs,
    path::{Component, MAIN_SEPARATOR, Path, PathBuf, is_separator},
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

/// Expands `~` and makes a path absolute, preserving the user-selected
/// filesystem location. `.` and `..` components are normalized so equivalent
/// addressings of one location compare and store identically; a `..` whose
/// preceding component exists on disk follows that component's real nature
/// (see [`parent_step`]).
pub fn absolutize(path: &Path) -> PathBuf {
    let expanded = expand_home(path);
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(&expanded),
            Err(_) => expanded,
        }
    };
    normalize_lexically(&absolute)
}

/// Removes `.` and applies `..` against its preceding component, keeping the
/// user's symlink aliases wherever `..` does not cross one. Each `..` follows
/// [`parent_step`]: popped lexically over directories and missing components,
/// resolved through the filesystem over directory symlinks, and preserved
/// over existing non-directories the OS would reject. Leading `..` components
/// of a relative path are kept; excess `..` at an absolute root is dropped,
/// matching how the OS resolves it.
fn normalize_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {},
            Component::ParentDir => {
                let last_is_normal = normalized
                    .components()
                    .next_back()
                    .is_some_and(|last| matches!(last, Component::Normal(_)));
                let last_is_parent = normalized
                    .components()
                    .next_back()
                    .is_some_and(|last| matches!(last, Component::ParentDir));
                if last_is_normal {
                    match parent_step(&normalized) {
                        ParentStep::Lexical => {
                            normalized.pop();
                        },
                        ParentStep::Resolved(parent) => normalized = parent,
                        ParentStep::Preserved => normalized.push(component.as_os_str()),
                    }
                } else if last_is_parent || !normalized.has_root() {
                    normalized.push(component.as_os_str());
                }
            },
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

/// How a `..` applies to the prefix before it.
enum ParentStep {
    /// A real directory: popping is exactly what the OS would do.
    Lexical,
    /// A symlink to a directory: `..` lands at the target's parent.
    Resolved(PathBuf),
    /// Anything else - missing, unreachable, or a non-directory: the OS
    /// rejects the traversal, so it is kept intact rather than rewritten to a
    /// valid but unrelated path.
    Preserved,
}

/// Classifies `..` against `prefix`. Only a real, reachable directory may be
/// traversed upward; a directory symlink resolves through the filesystem, and
/// everything else - missing, denied, or a non-directory - preserves its
/// traversal, because the OS would reject it (`ENOENT`, `EACCES`, `ENOTDIR`)
/// and normalizing it away could silently select a different workspace.
fn parent_step(prefix: &Path) -> ParentStep {
    let Ok(meta) = fs::symlink_metadata(prefix) else {
        return ParentStep::Preserved;
    };
    if meta.file_type().is_symlink() {
        return match prefix.canonicalize() {
            Ok(target) if target.is_dir() => match target.parent() {
                Some(parent) => ParentStep::Resolved(parent.to_path_buf()),
                // The target is the filesystem root, whose parent is itself.
                None => ParentStep::Resolved(target),
            },
            _ => ParentStep::Preserved,
        };
    }
    if meta.is_dir() {
        ParentStep::Lexical
    } else {
        ParentStep::Preserved
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
        assert_eq!(normalize(&link), target.canonicalize().unwrap());

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

    /// Dot components vanish; `..` over a real directory pops to its parent.
    #[test]
    fn absolutize_normalizes_dot_components() {
        assert_eq!(
            absolutize(Path::new("/tmp/./proj/./muster.yml")),
            PathBuf::from("/tmp/proj/muster.yml")
        );
        assert_eq!(
            absolutize(Path::new("/../etc/passwd")),
            PathBuf::from("/etc/passwd"),
            "excess parent components at the root are dropped"
        );
        let real = std::env::temp_dir().join(format!("muster-abs-dot-{}", std::process::id()));
        let sub = real.join("sub");
        fs::create_dir_all(&sub).unwrap();
        assert_eq!(
            absolutize(&sub.join("../muster.yml")),
            real.join("muster.yml"),
            "an existing directory pops"
        );
        fs::remove_dir_all(real).unwrap();
    }

    /// `..` over a missing component is preserved: the OS would fail the
    /// resolution with ENOENT, so it must not be normalized into a valid,
    /// unrelated path.
    #[test]
    fn missing_components_preserve_their_traversal() {
        assert_eq!(
            normalize_lexically(Path::new("/a/missing/../b")),
            PathBuf::from("/a/missing/../b")
        );
        assert_eq!(
            normalize_lexically(Path::new("../x/./y")),
            PathBuf::from("../x/y"),
            "leading parent components and dot removal are untouched"
        );
        assert_eq!(
            normalize_lexically(Path::new("../../x")),
            PathBuf::from("../../x")
        );
    }

    /// A search-permission mode removing all access, for the denied case.
    #[cfg(unix)]
    const LOCKED_MODE: u32 = 0o000;
    /// A normal directory mode, restored so cleanup can run.
    #[cfg(unix)]
    const OPEN_MODE: u32 = 0o755;

    #[cfg(unix)]
    /// `..` under an unsearchable directory is preserved instead of escaping
    /// lexically toward a different, accessible workspace.
    #[test]
    fn traversal_in_an_unsearchable_directory_is_preserved() {
        use std::os::unix::fs::PermissionsExt;

        let base = std::env::temp_dir().join(format!("muster-denied-{}", std::process::id()));
        let locked = base.join("locked");
        let inner = locked.join("inner");
        fs::create_dir_all(&inner).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(LOCKED_MODE)).unwrap();
        let probe = inner.join("x/../muster.yml");
        let normalized = normalize_lexically(&probe);

        fs::set_permissions(&locked, fs::Permissions::from_mode(OPEN_MODE)).unwrap();
        // Denied and missing prefixes both preserve, so this holds even for
        // root, which ignores the permission bits and sees the path missing.
        assert_eq!(normalized, probe, "the traversal survives untouched");
        fs::remove_dir_all(&base).unwrap();
    }

    /// `..` after an existing regular file is preserved: the OS rejects the
    /// traversal, so normalization must not rewrite it to a valid path.
    #[test]
    fn parent_of_a_regular_file_is_preserved() {
        let base = std::env::temp_dir().join(format!("muster-file-dotdot-{}", std::process::id()));
        let blocker = base.join("blocker");
        fs::create_dir_all(&base).unwrap();
        fs::write(&blocker, "").unwrap();

        assert_eq!(
            normalize_lexically(&blocker.join("../muster.yml")),
            blocker.join("../muster.yml"),
            "the invalid traversal survives for the OS to reject"
        );
        fs::remove_dir_all(base).unwrap();
    }

    /// `..` after an existing real directory still pops, matching the OS.
    #[test]
    fn parent_of_a_real_directory_pops() {
        let base = std::env::temp_dir().join(format!("muster-dir-dotdot-{}", std::process::id()));
        let sub = base.join("sub");
        fs::create_dir_all(&sub).unwrap();

        assert_eq!(
            normalize_lexically(&sub.join("../muster.yml")),
            base.join("muster.yml")
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[cfg(unix)]
    /// `..` across a real symlink resolves through the filesystem: the parent
    /// of the link's target, not of the link itself.
    #[test]
    fn parent_of_a_symlink_resolves_through_the_filesystem() {
        use std::os::unix::fs::symlink;

        let base = std::env::temp_dir().join(format!("muster-link-dotdot-{}", std::process::id()));
        let real_sub = base.join("real/sub");
        let link = base.join("link");
        fs::create_dir_all(&real_sub).unwrap();
        symlink(&real_sub, &link).unwrap();

        let resolved = normalize_lexically(&link.join("../muster.yml"));

        assert_eq!(
            resolved,
            base.join("real").canonicalize().unwrap().join("muster.yml"),
            "link/.. lands beside the link target, matching the OS"
        );
        fs::remove_dir_all(base).unwrap();
    }
}
