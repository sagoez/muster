use std::{
    fmt::{self, Display, Formatter},
    path::PathBuf,
};

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::value::ProcessName;

/// A stored working directory that leaves the project root, as only a hand
/// edit of the database can make: the process it belongs to reads, so it can
/// be changed or removed, but is never started.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct UnconfinedDir {
    /// The process it belongs to.
    name: ProcessName,
    /// The directory as stored.
    dir: PathBuf,
}

impl Display for UnconfinedDir {
    /// Which process, which directory, and the commands that fix it.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let name = self.name.shell_quoted();
        write!(
            f,
            "the stored working dir '{}' of {name} leaves the project root, so it is never \
             started; `muster set {name} --no-cwd` or `muster rm {name}` fixes it",
            self.dir.display()
        )
    }
}
