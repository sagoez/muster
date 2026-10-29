use std::path::PathBuf;

use super::process_flags::ProcessFlags;

/// `muster add`: a new process's definition.
#[derive(clap::Args)]
pub struct AddArgs {
    /// Its name, unique in the project.
    pub name: String,
    /// Its working directory, read from the current folder; stored relative
    /// to the project when inside it. Defaults to the project's folder.
    #[arg(long, value_name = "DIR")]
    pub cwd: Option<PathBuf>,
    /// A one-line description.
    #[arg(long, value_name = "TEXT")]
    pub description: Option<String>,
    #[command(flatten)]
    pub flags: ProcessFlags,
    /// Print the result as JSON.
    #[arg(long)]
    pub json: bool,
    /// The command, after `--`: one argument is a shell expression, several
    /// are joined as the shell split them.
    #[arg(last = true, required = true, value_name = "COMMAND")]
    pub command: Vec<String>,
}
