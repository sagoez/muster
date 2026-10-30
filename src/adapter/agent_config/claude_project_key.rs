use std::{
    fs,
    path::{Path, PathBuf},
};

/// A repository's git folder, or in a linked worktree the file naming it.
const GIT: &str = ".git";
/// What a worktree's `.git` file starts with, before the git folder.
const GITDIR_PREFIX: &str = "gitdir:";
/// The file in a worktree's git folder naming the repository's own.
const COMMONDIR: &str = "commondir";
/// The file in a worktree's git folder naming the worktree's `.git` file.
const GITDIR: &str = "gitdir";
/// The folder of a repository's git folder holding its worktrees'.
const WORKTREES: &str = "worktrees";

/// The folder Claude Code keys a project's local settings by in its
/// `.claude.json`, for `folder`, a canonical project folder: the root of
/// the git repository holding it, the main one for a linked worktree, else
/// `folder` itself. Claude Code reads that key when started anywhere in the
/// repository.
pub(super) fn claude_project_key(folder: &Path) -> PathBuf {
    match folder.ancestors().find(|dir| dir.join(GIT).exists()) {
        Some(root) => main_worktree(root).unwrap_or_else(|| root.to_path_buf()),
        None => folder.to_path_buf(),
    }
}

/// The main repository's folder when `root` is a linked worktree whose
/// `.git` file and git folder name each other, as Claude Code checks; none
/// for a repository's own root, a submodule, or anything it cannot confirm.
fn main_worktree(root: &Path) -> Option<PathBuf> {
    let dot_git = root.join(GIT);
    let named = fs::read_to_string(&dot_git).ok()?;
    let git_dir = root.join(named.trim().strip_prefix(GITDIR_PREFIX)?.trim());
    let common = git_dir.join(fs::read_to_string(git_dir.join(COMMONDIR)).ok()?.trim());
    let common = fs::canonicalize(common).ok()?;
    if fs::canonicalize(git_dir.parent()?).ok()? != common.join(WORKTREES) {
        return None;
    }
    let back = git_dir.join(fs::read_to_string(git_dir.join(GITDIR)).ok()?.trim());
    if fs::canonicalize(back).ok()? != fs::canonicalize(&dot_git).ok()? {
        return None;
    }
    if common.file_name()? == GIT {
        common.parent().map(Path::to_path_buf)
    } else {
        Some(common)
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    /// A git repository at `repo` with a linked worktree at `worktree`, laid
    /// out as `git worktree add` makes them.
    fn repository_with_worktree(repo: &Path, worktree: &Path) {
        let git_dir = repo.join(".git/worktrees/wt");
        fs::create_dir_all(&git_dir).unwrap();
        fs::create_dir_all(worktree).unwrap();
        fs::write(
            worktree.join(GIT),
            format!("gitdir: {}\n", git_dir.display()),
        )
        .unwrap();
        fs::write(git_dir.join(COMMONDIR), "../..\n").unwrap();
        fs::write(
            git_dir.join(GITDIR),
            format!("{}\n", worktree.join(GIT).display()),
        )
        .unwrap();
    }

    /// A folder in a repository keys by the repository's root, a linked
    /// worktree by its main repository's, a submodule by its own root, and
    /// a folder outside git by itself.
    #[test]
    fn the_key_is_the_repository_holding_the_project() {
        let dir = TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let repo = base.join("repo");
        let worktree = base.join("wt");
        repository_with_worktree(&repo, &worktree);
        fs::create_dir_all(repo.join("web/src")).unwrap();
        fs::create_dir_all(worktree.join("api")).unwrap();

        assert_eq!(claude_project_key(&repo.join("web/src")), repo);
        assert_eq!(claude_project_key(&repo), repo);
        assert_eq!(claude_project_key(&worktree.join("api")), repo);
        assert_eq!(claude_project_key(&worktree), repo);

        let module = repo.join("vendor/lib");
        fs::create_dir_all(&module).unwrap();
        fs::create_dir_all(repo.join(".git/modules/lib")).unwrap();
        fs::write(module.join(GIT), "gitdir: ../../.git/modules/lib\n").unwrap();
        assert_eq!(claude_project_key(&module), module);

        let plain = base.join("plain/sub");
        fs::create_dir_all(&plain).unwrap();
        assert_eq!(claude_project_key(&plain), plain);
    }
}
