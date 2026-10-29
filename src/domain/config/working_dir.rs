use std::{
    fmt::{self, Display, Formatter},
    path::PathBuf,
};

/// How a working directory that is the project root itself reads.
const PROJECT_ROOT: &str = "the project root";

/// A stored working directory as a person reads it.
pub(super) struct WorkingDir<'a>(pub(super) &'a Option<PathBuf>);

impl Display for WorkingDir<'_> {
    /// The folder, or [`PROJECT_ROOT`] for none.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(dir) => write!(f, "{}", dir.display()),
            None => f.write_str(PROJECT_ROOT),
        }
    }
}
