use std::{
    fs,
    path::{Path, PathBuf},
};

use super::working_dir_error::WorkingDirError;
use crate::domain::value::ProjectRoot;

/// Where a process's working directory sits relative to its project, with
/// symlinks resolved, so a folder reached through an alias of the project
/// still counts as inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkingDirPlace {
    /// The project's own folder.
    Root,
    /// A folder inside the project, relative to it.
    Inside(PathBuf),
    /// A folder outside the project, absolute.
    Outside(PathBuf),
}

impl WorkingDirPlace {
    /// Where `dir`, an absolute folder, sits relative to `root`.
    ///
    /// # Errors
    /// Returns [`WorkingDirError::NotADirectory`] when `dir` is not an
    /// existing directory, or [`WorkingDirError::Unrepresentable`] when it is
    /// not UTF-8.
    pub fn of(root: &ProjectRoot, dir: &Path) -> Result<Self, WorkingDirError> {
        let resolved = fs::canonicalize(dir)
            .ok()
            .filter(|resolved| resolved.is_dir())
            .ok_or_else(|| WorkingDirError::NotADirectory(dir.to_path_buf()))?;
        if resolved.to_str().is_none() {
            return Err(WorkingDirError::Unrepresentable(resolved));
        }
        let base = fs::canonicalize(root.as_ref()).unwrap_or_else(|_| root.as_ref().clone());
        Ok(match resolved.strip_prefix(&base) {
            Ok(inside) if inside.as_os_str().is_empty() => Self::Root,
            Ok(inside) => Self::Inside(inside.to_path_buf()),
            Err(_) => Self::Outside(resolved),
        })
    }

    /// The working directory to store: none for the project's own folder,
    /// relative inside it, absolute outside it.
    pub fn stored(self) -> Option<PathBuf> {
        match self {
            Self::Root => None,
            Self::Inside(dir) | Self::Outside(dir) => Some(dir),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use tempfile::TempDir;

    use super::*;

    /// The root itself is the root, a folder inside it relative even through
    /// an alias, and one outside it absolute; a missing folder or a file is
    /// refused.
    #[test]
    fn a_working_dir_is_placed_relative_to_the_root() {
        let scratch = TempDir::new().unwrap();
        let base = scratch.path().canonicalize().unwrap();
        fs::create_dir_all(base.join("shop/frontend")).unwrap();
        fs::create_dir_all(base.join("tools")).unwrap();
        fs::write(base.join("shop/notes.txt"), "").unwrap();
        symlink(base.join("shop"), base.join("alias")).unwrap();
        let root = ProjectRoot::try_new(base.join("shop")).unwrap();

        assert_eq!(
            WorkingDirPlace::of(&root, &base.join("shop")),
            Ok(WorkingDirPlace::Root)
        );
        assert_eq!(
            WorkingDirPlace::of(&root, &base.join("alias/frontend")),
            Ok(WorkingDirPlace::Inside(PathBuf::from("frontend")))
        );
        assert_eq!(
            WorkingDirPlace::of(&root, &base.join("tools")),
            Ok(WorkingDirPlace::Outside(base.join("tools")))
        );
        assert!(matches!(
            WorkingDirPlace::of(&root, &base.join("shop/notes.txt")),
            Err(WorkingDirError::NotADirectory(_))
        ));
        assert!(matches!(
            WorkingDirPlace::of(&root, &base.join("missing")),
            Err(WorkingDirError::NotADirectory(_))
        ));
    }

    /// Only the root is stored as none; a folder keeps its placed path.
    #[test]
    fn a_placed_dir_stores_its_path() {
        assert_eq!(WorkingDirPlace::Root.stored(), None);
        assert_eq!(
            WorkingDirPlace::Inside(PathBuf::from("web")).stored(),
            Some(PathBuf::from("web"))
        );
    }
}
