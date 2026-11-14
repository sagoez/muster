use std::path::{Path, PathBuf};

use muster_domain::constants::STDIO_PATH;

use crate::{
    cli::{command_line::invocation::Invocation, failure::error::CliError},
    exchange::STACK_FILE,
};

/// Where `export` writes a stack file and `import` reads one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StackPath {
    /// stdout for `export`, stdin for `import`.
    Stdio,
    /// This file.
    File(PathBuf),
}

impl StackPath {
    /// Where `file` points: stdio for `-`, the stack file in `root` when
    /// absent, else the path read against the current directory.
    ///
    /// # Errors
    /// Returns [`CliError::CurrentDir`] when the current directory is needed
    /// and cannot be read.
    pub fn of(
        invocation: &Invocation<'_>,
        root: &Path,
        file: Option<&str>,
    ) -> Result<Self, CliError> {
        Ok(match file {
            Some(STDIO_PATH) => Self::Stdio,
            Some(path) => Self::File(invocation.in_cwd(Path::new(path))?),
            None => Self::File(root.join(STACK_FILE)),
        })
    }
}
