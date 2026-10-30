use std::fmt::{self, Display, Formatter};

use crate::domain::value::ProjectName;

/// Why a folder may not be registered as a project: it would claim the whole
/// machine, the user's home, or a registered project's folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefusedFolder {
    /// The filesystem root.
    FilesystemRoot,
    /// The user's home folder.
    Home,
    /// A folder above the user's home folder.
    AboveHome,
    /// A folder holding the registered project named here.
    HoldsProject(ProjectName),
}

impl Display for RefusedFolder {
    /// The reason, phrased to follow "it is".
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::FilesystemRoot => f.write_str("the filesystem root"),
            Self::Home => f.write_str("your home folder"),
            Self::AboveHome => f.write_str("above your home folder"),
            Self::HoldsProject(name) => write!(f, "a folder holding the project '{name}'"),
        }
    }
}
