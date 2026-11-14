use std::path::PathBuf;

use getset::{CopyGetters, Getters};
use muster_domain::process::ProcessIcon;

use super::{args::icon_parser, process_flags::ProcessFlags};

/// `muster add`: a new process's definition.
#[derive(clap::Args, Getters, CopyGetters)]
pub struct AddArgs {
    /// Its name, unique in the project.
    #[getset(get = "pub")]
    name: String,
    /// Its working directory, read from the current folder; stored relative
    /// to the project when inside it. Defaults to the project's folder.
    #[arg(long, value_name = "DIR")]
    #[getset(get = "pub")]
    cwd: Option<PathBuf>,
    /// A one-line description.
    #[arg(long, value_name = "TEXT")]
    #[getset(get = "pub")]
    description: Option<String>,
    /// The icon `muster view` shows it with.
    #[arg(long, value_name = "NAME", value_parser = icon_parser())]
    #[getset(get_copy = "pub")]
    icon: Option<ProcessIcon>,
    #[command(flatten)]
    #[getset(get = "pub")]
    flags: ProcessFlags,
    /// Print the result as JSON.
    #[arg(long)]
    #[getset(get_copy = "pub")]
    json: bool,
    /// The command, after `--`: one argument is a shell expression, several
    /// are joined as the shell split them.
    #[arg(last = true, required = true, value_name = "COMMAND")]
    #[getset(get = "pub")]
    command: Vec<String>,
}
