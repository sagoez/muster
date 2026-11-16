use std::path::Path;

use strum::Display;

/// What clears a refused folder in `/tmp`, whose fixed path any user could
/// have made first.
const TMP_REMEDY: &str = "have its owner or an administrator remove it, or reboot to clear /tmp";
/// The mode a socket folder must have, as `chmod` takes it.
const PRIVATE_MODE: &str = "700";

/// Where a holder socket folder lies, which decides what clears a refusal
/// of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
#[strum(serialize_all = "kebab-case")]
pub enum SocketDirPlace {
    /// In `/tmp`, at a fixed path another user may have made first.
    Tmp,
    /// In the state directory, where an earlier muster bound holder sockets.
    StateDir,
}

impl SocketDirPlace {
    /// What the user can do to have the folder `dir` in this place used.
    pub fn remedy(self, dir: &Path) -> String {
        match self {
            Self::Tmp => TMP_REMEDY.to_string(),
            Self::StateDir => format!(
                "make it a directory you own with mode {PRIVATE_MODE}, as `chmod {PRIVATE_MODE} \
                 {}` does once you own it",
                dir.display()
            ),
        }
    }
}
