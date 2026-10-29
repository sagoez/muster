use std::path::PathBuf;

use super::{
    args::command_line,
    process_flags::{ProcessFlags, READY_GROUP},
};
use crate::domain::value::{CommandLine, ProcessRef};

/// `muster set`: the changes to a process's definition.
#[derive(clap::Args)]
pub struct SetArgs {
    /// The process, as `muster ps` lists it.
    pub name: ProcessRef,
    /// A new command, as one shell expression.
    #[arg(long, value_name = "CMD", value_parser = command_line, conflicts_with = "words")]
    pub command: Option<CommandLine>,
    /// A new working directory, read from the current folder.
    #[arg(long, value_name = "DIR", conflicts_with = "no_cwd")]
    pub cwd: Option<PathBuf>,
    /// Run in the project's folder again.
    #[arg(long)]
    pub no_cwd: bool,
    /// A new one-line description.
    #[arg(long, value_name = "TEXT", conflicts_with = "no_description")]
    pub description: Option<String>,
    /// Remove the description.
    #[arg(long)]
    pub no_description: bool,
    /// Remove an environment variable (repeatable); one that is not set is
    /// left alone.
    #[arg(long = "unset-env", value_name = "KEY")]
    pub unset_env: Vec<String>,
    /// Remove the readiness check.
    #[arg(long, conflicts_with = READY_GROUP)]
    pub no_ready: bool,
    #[command(flatten)]
    pub flags: ProcessFlags,
    /// Print the result as JSON.
    #[arg(long)]
    pub json: bool,
    /// A new command, after `--`, joined as `muster add` joins it.
    #[arg(last = true, value_name = "COMMAND")]
    pub words: Vec<String>,
}
