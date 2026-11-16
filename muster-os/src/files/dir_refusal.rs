//! Why a folder muster would keep a socket in was refused, and what clears
//! it: the one place every refusal's advice comes from.

use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

use super::private_dir::{FALLBACK_SOCKET_ROOT, PRIVATE_DIR_MODE, is_current_user, tmp_socket_dir};

/// Advice for a refused entry that is a file, or a link to nothing that is
/// a folder, before the command that removes it.
const REMOVE_ENTRY: &str = "it is not a real directory, so remove it with";
/// Advice for a refused link to a folder, before the commands that put the
/// folder in its place.
const REPLACE_LINK: &str =
    "it is a link, not a real directory, so put the folder it leads to in its place with";
/// Advice for a refused folder another user owns.
const HAVE_IT_REMOVED: &str = "have its owner or an administrator remove it";
/// What a reboot does for a folder another user left in `/tmp`.
const OR_REBOOT: &str = ", or reboot to clear";
/// Says the refused folder is where every holder socket goes.
const WHERE_PROCESS_SOCKETS_GO: &str =
    "is where every process's socket goes, so it must be fixed rather than avoided";

/// Why a socket folder was refused, which decides what clears it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirRefusal {
    /// A real directory the user owns whose mode lets others in.
    WrongMode,
    /// A link the user owns to the folder at the path it holds, which must
    /// take the link's place, since what is kept in it must not be lost.
    Link(PathBuf),
    /// A file, or a link to nothing that is a folder, that the user owns
    /// where a directory should be.
    NotADirectory,
    /// Something another user owns at a path in `/tmp`, which a reboot
    /// clears.
    ForeignInTmp,
    /// Something another user owns elsewhere, which no reboot clears.
    Foreign,
}

impl DirRefusal {
    /// Why `dir`, already found not private, was refused, with the uid that
    /// owns it, both read from one look at it that never follows a link. An
    /// entry whose owner cannot be read counts as another user's.
    pub fn of(dir: &Path) -> (Self, Option<u32>) {
        let Ok(metadata) = fs::symlink_metadata(dir) else {
            return (Self::foreign_at(dir), None);
        };
        let owner = Some(metadata.uid());
        let refusal = if !is_current_user(metadata.uid()) {
            Self::foreign_at(dir)
        } else if metadata.is_dir() {
            Self::WrongMode
        } else if let Some(target) = linked_folder(dir) {
            Self::Link(target)
        } else {
            Self::NotADirectory
        };
        (refusal, owner)
    }

    /// The refusal of another user's entry at `dir`.
    fn foreign_at(dir: &Path) -> Self {
        if dir.starts_with(FALLBACK_SOCKET_ROOT) {
            Self::ForeignInTmp
        } else {
            Self::Foreign
        }
    }

    /// Whether another user owns the refused entry, so the user cannot
    /// clear it alone.
    pub fn is_foreign(&self) -> bool {
        matches!(self, Self::ForeignInTmp | Self::Foreign)
    }

    /// What clears this refusal of `dir`, every path in a command quoted for
    /// a shell. Only when `dir` is `/tmp/muster-<uid>` itself does it add
    /// that every holder socket goes there.
    pub fn remedy(&self, dir: &Path) -> String {
        let quoted = shell_quoted(dir);
        let remedy = match self {
            Self::WrongMode => {
                format!("make it private with `chmod {PRIVATE_DIR_MODE:o} {quoted}`")
            },
            Self::Link(target) => format!(
                "{REPLACE_LINK} `rm {quoted} && mv {} {quoted}`",
                shell_quoted(target)
            ),
            Self::NotADirectory => format!("{REMOVE_ENTRY} `rm {quoted}`"),
            Self::ForeignInTmp => format!("{HAVE_IT_REMOVED}{OR_REBOOT} {FALLBACK_SOCKET_ROOT}"),
            Self::Foreign => HAVE_IT_REMOVED.to_string(),
        };
        if dir == tmp_socket_dir() {
            return format!("{remedy}; {} {WHERE_PROCESS_SOCKETS_GO}", dir.display());
        }
        remedy
    }
}

/// The folder the link at `link` leads to, resolved, when it leads to one.
fn linked_folder(link: &Path) -> Option<PathBuf> {
    fs::canonicalize(link).ok().filter(|target| target.is_dir())
}

/// `path` quoted so a shell reads it as one word, or as it is when it holds
/// a byte no quoting can carry.
fn shell_quoted(path: &Path) -> String {
    let text = path.to_string_lossy();
    shlex::try_quote(&text).map_or_else(|_| text.to_string(), |quoted| quoted.into_owned())
}

#[cfg(test)]
mod tests {
    use std::{
        fs::Permissions,
        os::unix::fs::{PermissionsExt, symlink},
    };

    use tempfile::TempDir;

    use super::*;
    use crate::{private_dir::owner_of, test_scratch::SHARED_DIR_MODE};

    /// A folder outside `/tmp` that root owns.
    const ROOT_OWNED_DIR: &str = "/";
    /// What advice to fix a mode says.
    const CHMOD: &str = "chmod 700";
    /// What advice to reboot says.
    const REBOOT: &str = "reboot";
    /// A folder name with a space, as macOS's `Application Support` has.
    const SPACED: &str = "Application Support";
    /// A refused folder a test makes.
    const REFUSED: &str = "refused";
    /// A file a test plants where a folder should be.
    const PLANTED_FILE: &str = "file";
    /// A link a test plants where a folder should be.
    const PLANTED_LINK: &str = "link";
    /// A folder below `/tmp/muster-<uid>`, as a state directory's is.
    const KEY_FOLDER: &str = "key";

    /// Whether `path` belongs to another user here, as it does on a usual
    /// host but not in every sandbox.
    fn is_foreign(path: &Path) -> bool {
        owner_of(path).is_some_and(|owner| !is_current_user(owner))
    }

    /// A folder of one's own with a wrong mode is fixed with `chmod`,
    /// naming it, and never removed or rebooted away.
    #[test]
    fn a_folder_of_ones_own_is_fixed_with_chmod() {
        let scratch = TempDir::new().unwrap();
        let shared = scratch.path().join(REFUSED);
        fs::create_dir(&shared).unwrap();
        fs::set_permissions(&shared, Permissions::from_mode(SHARED_DIR_MODE)).unwrap();

        let (refusal, owner) = DirRefusal::of(&shared);
        let remedy = refusal.remedy(&shared);

        assert_eq!(refusal, DirRefusal::WrongMode);
        assert_eq!(owner, owner_of(&shared));
        assert!(
            remedy.contains(&format!("{CHMOD} {}", shared.display())),
            "{remedy}"
        );
        assert!(
            !remedy.contains(HAVE_IT_REMOVED) && !remedy.contains(REBOOT),
            "{remedy}"
        );
    }

    /// A path with a space is quoted in the command, so pasting it acts on
    /// that one path.
    #[test]
    fn a_path_with_a_space_is_quoted() {
        let scratch = TempDir::new().unwrap();
        let spaced = scratch.path().join(SPACED);

        let remedy = DirRefusal::WrongMode.remedy(&spaced);

        assert!(
            remedy.contains(&format!("{CHMOD} '{}'", spaced.display())),
            "{remedy}"
        );
    }

    /// A file, or a link to nothing, of one's own where a folder should be
    /// is removed, never chmodded, which would not make it a folder and
    /// follows a link.
    #[test]
    fn a_file_or_dangling_link_of_ones_own_is_removed() {
        let scratch = TempDir::new().unwrap();
        let file = scratch.path().join(PLANTED_FILE);
        fs::write(&file, b"").unwrap();
        let link = scratch.path().join(PLANTED_LINK);
        symlink(scratch.path().join(REFUSED), &link).unwrap();

        for entry in [&file, &link] {
            let (refusal, _) = DirRefusal::of(entry);
            let remedy = refusal.remedy(entry);

            assert_eq!(refusal, DirRefusal::NotADirectory, "{}", entry.display());
            assert!(
                remedy.contains(&format!("rm {}", entry.display())),
                "{remedy}"
            );
            assert!(!remedy.contains(CHMOD), "{remedy}");
        }
    }

    /// A link to a folder is replaced by that folder, never just removed,
    /// so what is kept in it, holder records under `holders` among them, is
    /// not cut off.
    #[test]
    fn a_link_to_a_folder_is_replaced_by_the_folder() {
        let scratch = TempDir::new().unwrap();
        let target = scratch.path().join(REFUSED);
        fs::create_dir(&target).unwrap();
        let link = scratch.path().join(PLANTED_LINK);
        symlink(&target, &link).unwrap();
        let resolved = fs::canonicalize(&target).unwrap();

        let (refusal, _) = DirRefusal::of(&link);
        let remedy = refusal.remedy(&link);

        assert_eq!(refusal, DirRefusal::Link(resolved.clone()));
        assert!(
            remedy.contains(&format!(
                "rm {} && mv {} {}",
                link.display(),
                resolved.display(),
                link.display()
            )),
            "{remedy}"
        );
    }

    /// Another user's folder in `/tmp` is removed by them or by a reboot;
    /// one elsewhere only by them, with no word of a reboot.
    #[test]
    fn another_users_folder_is_theirs_to_remove() {
        let in_tmp = Path::new(FALLBACK_SOCKET_ROOT);
        let elsewhere = Path::new(ROOT_OWNED_DIR);
        if is_foreign(in_tmp) {
            assert_eq!(DirRefusal::of(in_tmp).0, DirRefusal::ForeignInTmp);
        }
        if is_foreign(elsewhere) {
            assert_eq!(DirRefusal::of(elsewhere).0, DirRefusal::Foreign);
        }

        let tmp_remedy = DirRefusal::ForeignInTmp.remedy(in_tmp);
        assert!(
            tmp_remedy.contains(HAVE_IT_REMOVED) && tmp_remedy.contains(REBOOT),
            "{tmp_remedy}"
        );
        let remedy = DirRefusal::Foreign.remedy(elsewhere);
        assert!(remedy.contains(HAVE_IT_REMOVED), "{remedy}");
        assert!(!remedy.contains(REBOOT), "{remedy}");
    }

    /// Only a refusal of `/tmp/muster-<uid>` itself says every process's
    /// socket goes there.
    #[test]
    fn only_the_tmp_folder_itself_is_said_to_take_process_sockets() {
        let scratch = TempDir::new().unwrap();
        let tmp = tmp_socket_dir();

        for refusal in [
            DirRefusal::WrongMode,
            DirRefusal::Link(scratch.path().to_path_buf()),
            DirRefusal::NotADirectory,
            DirRefusal::ForeignInTmp,
            DirRefusal::Foreign,
        ] {
            assert!(
                refusal.remedy(&tmp).contains(WHERE_PROCESS_SOCKETS_GO),
                "{refusal:?}"
            );
            for other in [scratch.path().to_path_buf(), tmp.join(KEY_FOLDER)] {
                assert!(
                    !refusal.remedy(&other).contains(WHERE_PROCESS_SOCKETS_GO),
                    "{refusal:?}"
                );
            }
        }
    }
}
