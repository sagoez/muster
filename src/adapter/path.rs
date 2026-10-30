use std::{
    env, fs,
    path::{Component, Path, PathBuf},
};

use directories::BaseDirs;

use crate::domain::{config::ConfigError, value::ProjectRoot};

/// Leading path component expanded to the user's home directory.
const HOME_PREFIX: &str = "~";
/// Marks a project value as a folder when it starts with one of these.
const FOLDER_PREFIXES: [char; 2] = ['.', '~'];
/// Marks a project value as a folder when it contains it anywhere.
const PATH_SEPARATOR: char = '/';

/// Whether a project value someone typed names a folder (it contains `/` or
/// starts with `.` or `~`) rather than a registered project's name.
pub fn names_a_folder(value: &str) -> bool {
    value.contains(PATH_SEPARATOR) || value.starts_with(FOLDER_PREFIXES)
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

/// The user's home folder, canonicalized when it exists, so it compares with
/// a [`ProjectRoot`].
pub fn home_dir() -> Option<PathBuf> {
    BaseDirs::new().map(|dirs| {
        let home = dirs.home_dir();
        home.canonicalize().unwrap_or_else(|_| home.to_path_buf())
    })
}

/// The project folder `dir` names: `~` expanded, canonicalized, and required
/// to be an existing directory whose path is UTF-8, since the store keeps
/// roots as text.
///
/// # Errors
/// Returns [`ConfigError::RootNotADirectory`] when `dir` is missing or is not
/// a directory, and [`ConfigError::RootNotUtf8`] when its canonical path is
/// not UTF-8.
pub fn project_root(dir: &Path) -> Result<ProjectRoot, ConfigError> {
    let refused = || ConfigError::RootNotADirectory {
        path: dir.to_path_buf(),
    };
    let canonical = expand_home(dir).canonicalize().map_err(|_| refused())?;
    if !canonical.is_dir() {
        return Err(refused());
    }
    if canonical.to_str().is_none() {
        return Err(ConfigError::RootNotUtf8 { path: canonical });
    }
    ProjectRoot::try_new(canonical).map_err(|_| refused())
}

/// Where absolute `path` leads with every symlink followed: its deepest
/// existing ancestor canonicalized, with the missing rest appended as written.
pub fn resolved(path: &Path) -> PathBuf {
    path.ancestors()
        .find_map(|ancestor| {
            let rest = path.strip_prefix(ancestor).ok()?;
            ancestor.canonicalize().ok().map(|real| real.join(rest))
        })
        .unwrap_or_else(|| path.to_path_buf())
}

/// Expands `~` and makes a path absolute, preserving the user-selected
/// filesystem location. `.` and `..` components are normalized so equivalent
/// addressings of one location compare and store identically; a `..` whose
/// preceding component exists on disk follows that component's real nature
/// (see `parent_step`).
pub fn absolutize(path: &Path) -> PathBuf {
    let expanded = expand_home(path);
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        match env::current_dir() {
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

#[cfg(test)]
mod tests {
    use std::{
        os::unix::fs::{PermissionsExt, symlink},
        process,
    };

    use tempfile::TempDir;

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

    /// A folder reached through a relative path or a symlink alias is one
    /// project root: the canonical folder.
    #[test]
    fn project_root_canonicalizes_aliases_of_one_folder() {
        let scratch = TempDir::new().unwrap();
        let real = scratch.path().join("real");
        let link = scratch.path().join("link");
        fs::create_dir(&real).unwrap();
        symlink(&real, &link).unwrap();
        let canonical = real.canonicalize().unwrap();

        assert_eq!(project_root(&link).unwrap().as_ref(), &canonical);
        assert_eq!(project_root(&real.join(".")).unwrap().as_ref(), &canonical);
        assert_eq!(
            project_root(Path::new(".")).unwrap().as_ref(),
            &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .canonicalize()
                .unwrap(),
            "a relative folder resolves against the working directory"
        );
    }

    /// A missing folder or a file is not a project root.
    #[test]
    fn project_root_refuses_a_missing_folder_or_a_file() {
        let scratch = TempDir::new().unwrap();
        let file = scratch.path().join("file");
        fs::write(&file, "").unwrap();

        for refused in [scratch.path().join("missing"), file] {
            assert!(matches!(
                project_root(&refused),
                Err(ConfigError::RootNotADirectory { path }) if path == refused
            ));
        }
    }

    /// A folder whose path is not UTF-8 is refused as such, since the store
    /// keeps roots as text.
    #[cfg(target_os = "linux")]
    #[test]
    fn project_root_refuses_a_path_that_is_not_utf8() {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

        let scratch = TempDir::new().unwrap();
        let folder = scratch.path().join(OsStr::from_bytes(b"caf\xe9"));
        fs::create_dir(&folder).unwrap();

        assert!(matches!(
            project_root(&folder),
            Err(ConfigError::RootNotUtf8 { .. })
        ));
    }

    /// An absolute location retains a symlink alias.
    #[test]
    fn absolutize_preserves_a_symlink_path() {
        let dir = env::temp_dir().join(format!("muster-path-link-{}", process::id()));
        let target = dir.join("target.yml");
        let link = dir.join("muster.yml");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&target, "").unwrap();
        symlink(&target, &link).unwrap();

        assert_eq!(absolutize(&link), link);

        fs::remove_dir_all(dir).unwrap();
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
        let real = env::temp_dir().join(format!("muster-abs-dot-{}", process::id()));
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
    const LOCKED_MODE: u32 = 0o000;
    /// A normal directory mode, restored so cleanup can run.
    const OPEN_MODE: u32 = 0o755;

    /// `..` under an unsearchable directory is preserved instead of escaping
    /// lexically toward a different, accessible workspace.
    #[test]
    fn traversal_in_an_unsearchable_directory_is_preserved() {
        let base = env::temp_dir().join(format!("muster-denied-{}", process::id()));
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
        let base = env::temp_dir().join(format!("muster-file-dotdot-{}", process::id()));
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
        let base = env::temp_dir().join(format!("muster-dir-dotdot-{}", process::id()));
        let sub = base.join("sub");
        fs::create_dir_all(&sub).unwrap();

        assert_eq!(
            normalize_lexically(&sub.join("../muster.yml")),
            base.join("muster.yml")
        );
        fs::remove_dir_all(base).unwrap();
    }

    /// `..` across a real symlink resolves through the filesystem: the parent
    /// of the link's target, not of the link itself.
    #[test]
    fn parent_of_a_symlink_resolves_through_the_filesystem() {
        let base = env::temp_dir().join(format!("muster-link-dotdot-{}", process::id()));
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
