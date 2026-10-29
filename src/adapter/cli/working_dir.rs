use std::path::{Path, PathBuf};

use super::error::CliError;
use crate::domain::value::ProjectRoot;

/// The working directory to store for `dir`, an absolute folder: none for
/// the project's own folder, relative to it when inside it, absolute
/// otherwise. Symlinks are resolved first, so a folder reached through an
/// alias of the project is still stored relative to it.
///
/// # Errors
/// Returns [`CliError::NotADirectory`] when `dir` is not an existing
/// directory, or [`CliError::UnrepresentablePath`] when it is not UTF-8.
pub fn stored_working_dir(root: &ProjectRoot, dir: &Path) -> Result<Option<PathBuf>, CliError> {
    let resolved = dir
        .canonicalize()
        .ok()
        .filter(|resolved| resolved.is_dir())
        .ok_or_else(|| CliError::NotADirectory(dir.to_path_buf()))?;
    if resolved.to_str().is_none() {
        return Err(CliError::UnrepresentablePath(resolved));
    }
    Ok(match root.relative(&resolved) {
        Some(relative) if relative.as_os_str().is_empty() => None,
        Some(relative) => Some(relative),
        None => Some(resolved),
    })
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::symlink};

    use tempfile::TempDir;

    use super::*;

    /// The root itself is stored as none, a folder inside it relative, and
    /// one outside it absolute; a missing folder or a file is refused.
    #[test]
    fn a_working_dir_is_stored_relative_to_the_root_when_inside() {
        let scratch = TempDir::new().unwrap();
        let base = scratch.path().canonicalize().unwrap();
        fs::create_dir_all(base.join("shop/frontend")).unwrap();
        fs::create_dir_all(base.join("tools")).unwrap();
        fs::write(base.join("shop/notes.txt"), "").unwrap();
        symlink(base.join("shop"), base.join("alias")).unwrap();
        let root = ProjectRoot::try_new(base.join("shop")).unwrap();

        assert_eq!(stored_working_dir(&root, &base.join("shop")).unwrap(), None);
        assert_eq!(
            stored_working_dir(&root, &base.join("alias/frontend")).unwrap(),
            Some(PathBuf::from("frontend"))
        );
        assert_eq!(
            stored_working_dir(&root, &base.join("tools")).unwrap(),
            Some(base.join("tools"))
        );
        assert!(matches!(
            stored_working_dir(&root, &base.join("shop/notes.txt")),
            Err(CliError::NotADirectory(_))
        ));
        assert!(matches!(
            stored_working_dir(&root, &base.join("missing")),
            Err(CliError::NotADirectory(_))
        ));
    }
}
