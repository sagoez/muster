//! Why a folder muster would keep a socket in was refused, and what clears
//! it: the one place every refusal's advice comes from.

use std::{fs, os::unix::fs::MetadataExt, path::Path};

use super::private_dir::{FALLBACK_SOCKET_ROOT, PRIVATE_DIR_MODE, is_current_user, tmp_socket_dir};

/// Advice for a refused entry that is a file or a link, not a directory.
const REMOVE_ENTRY: &str = "it is not a real directory, so remove it with `rm";
/// Advice for a refused folder another user owns.
const HAVE_IT_REMOVED: &str = "have its owner or an administrator remove it";
/// What a reboot does for a folder another user left in `/tmp`.
const OR_REBOOT: &str = ", or reboot to clear";
/// Says the refused folder is the one every holder socket lives in.
const HOLDS_PROCESS_SOCKETS: &str =
    "holds your running processes' sockets, so it must be fixed rather than avoided";

/// Why a socket folder was refused, which decides what clears it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirRefusal {
    /// A real directory the user owns whose mode lets others in.
    WrongMode,
    /// A file or a link the user owns, where a directory should be.
    NotADirectory,
    /// Something another user owns at a path in `/tmp`, which a reboot
    /// clears.
    ForeignInTmp,
    /// Something another user owns elsewhere, which no reboot clears.
    Foreign,
}

impl DirRefusal {
    /// Why `dir`, already found not private, was refused, read without
    /// following a link. An entry whose owner cannot be read counts as
    /// another user's.
    pub fn of(dir: &Path) -> Self {
        match fs::symlink_metadata(dir) {
            Ok(metadata) if is_current_user(metadata.uid()) => {
                if metadata.is_dir() {
                    Self::WrongMode
                } else {
                    Self::NotADirectory
                }
            },
            _ if dir.starts_with(FALLBACK_SOCKET_ROOT) => Self::ForeignInTmp,
            _ => Self::Foreign,
        }
    }

    /// Whether another user owns the refused entry, so the user cannot
    /// clear it alone.
    pub fn is_foreign(self) -> bool {
        matches!(self, Self::ForeignInTmp | Self::Foreign)
    }

    /// What clears this refusal of `dir`. Only when `dir` is
    /// `/tmp/muster-<uid>` itself does it add that every holder socket lives
    /// there.
    pub fn remedy(self, dir: &Path) -> String {
        let shown = dir.display();
        let remedy = match self {
            Self::WrongMode => format!("make it private with `chmod {PRIVATE_DIR_MODE:o} {shown}`"),
            Self::NotADirectory => format!("{REMOVE_ENTRY} {shown}`"),
            Self::ForeignInTmp => format!("{HAVE_IT_REMOVED}{OR_REBOOT} {FALLBACK_SOCKET_ROOT}"),
            Self::Foreign => HAVE_IT_REMOVED.to_string(),
        };
        if dir == tmp_socket_dir() {
            return format!("{remedy}; {shown} {HOLDS_PROCESS_SOCKETS}");
        }
        remedy
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs::Permissions,
        os::unix::fs::{PermissionsExt, symlink},
    };

    use rustix::process::geteuid;
    use tempfile::TempDir;

    use super::*;

    /// A directory mode other users can enter.
    const SHARED_DIR_MODE: u32 = 0o755;
    /// A folder outside `/tmp` that root owns.
    const ROOT_OWNED_DIR: &str = "/";
    /// What advice to fix a mode says.
    const CHMOD: &str = "chmod 700";
    /// What advice to reboot says.
    const REBOOT: &str = "reboot";

    /// A folder of one's own with a wrong mode is fixed with `chmod`,
    /// naming it, and never removed or rebooted away.
    #[test]
    fn a_folder_of_ones_own_is_fixed_with_chmod() {
        let scratch = TempDir::new().unwrap();
        let shared = scratch.path().join("shared");
        fs::create_dir(&shared).unwrap();
        fs::set_permissions(&shared, Permissions::from_mode(SHARED_DIR_MODE)).unwrap();

        let refusal = DirRefusal::of(&shared);
        let remedy = refusal.remedy(&shared);

        assert_eq!(refusal, DirRefusal::WrongMode);
        assert!(
            remedy.contains(&format!("{CHMOD} {}", shared.display())),
            "{remedy}"
        );
        assert!(
            !remedy.contains(HAVE_IT_REMOVED) && !remedy.contains(REBOOT),
            "{remedy}"
        );
    }

    /// A file or a link of one's own where a folder should be is removed,
    /// never chmodded, which would not make it a folder and follows a link.
    #[test]
    fn a_file_or_link_of_ones_own_is_removed() {
        let scratch = TempDir::new().unwrap();
        let file = scratch.path().join("file");
        fs::write(&file, b"").unwrap();
        let link = scratch.path().join("link");
        symlink(scratch.path(), &link).unwrap();

        for entry in [&file, &link] {
            let refusal = DirRefusal::of(entry);
            let remedy = refusal.remedy(entry);

            assert_eq!(refusal, DirRefusal::NotADirectory, "{}", entry.display());
            assert!(
                remedy.contains(&format!("rm {}", entry.display())),
                "{remedy}"
            );
            assert!(!remedy.contains(CHMOD), "{remedy}");
        }
    }

    /// Another user's folder in `/tmp` is removed by them or by a reboot;
    /// one elsewhere only by them, with no word of a reboot.
    #[test]
    fn another_users_folder_is_theirs_to_remove() {
        if geteuid().is_root() {
            return;
        }
        let in_tmp = Path::new(FALLBACK_SOCKET_ROOT);
        let elsewhere = Path::new(ROOT_OWNED_DIR);

        assert_eq!(DirRefusal::of(in_tmp), DirRefusal::ForeignInTmp);
        assert_eq!(DirRefusal::of(elsewhere), DirRefusal::Foreign);
        let tmp_remedy = DirRefusal::ForeignInTmp.remedy(in_tmp);
        assert!(
            tmp_remedy.contains(HAVE_IT_REMOVED) && tmp_remedy.contains(REBOOT),
            "{tmp_remedy}"
        );
        let remedy = DirRefusal::Foreign.remedy(elsewhere);
        assert!(remedy.contains(HAVE_IT_REMOVED), "{remedy}");
        assert!(!remedy.contains(REBOOT), "{remedy}");
    }

    /// Only a refusal of `/tmp/muster-<uid>` itself says it holds the
    /// running processes' sockets.
    #[test]
    fn only_the_tmp_folder_itself_is_said_to_hold_process_sockets() {
        let scratch = TempDir::new().unwrap();
        let tmp = tmp_socket_dir();

        for refusal in [
            DirRefusal::WrongMode,
            DirRefusal::NotADirectory,
            DirRefusal::ForeignInTmp,
            DirRefusal::Foreign,
        ] {
            assert!(
                refusal.remedy(&tmp).contains(HOLDS_PROCESS_SOCKETS),
                "{refusal:?}"
            );
            assert!(
                !refusal
                    .remedy(scratch.path())
                    .contains(HOLDS_PROCESS_SOCKETS),
                "{refusal:?}"
            );
            assert!(
                !refusal
                    .remedy(&tmp.join("key"))
                    .contains(HOLDS_PROCESS_SOCKETS),
                "{refusal:?}"
            );
        }
    }
}
