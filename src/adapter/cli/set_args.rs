use std::path::PathBuf;

use getset::{CopyGetters, Getters};

use super::{
    args::command_line,
    process_flags::{ProcessFlags, READY_GROUP},
};
use crate::domain::value::{CommandLine, ProcessRef};

/// `muster set`: the changes to a process's definition.
#[derive(clap::Args, Getters, CopyGetters)]
pub struct SetArgs {
    /// The process, as `muster ps` lists it.
    #[getset(get = "pub")]
    name: ProcessRef,
    /// A new command, as one shell expression.
    #[arg(long, value_name = "CMD", value_parser = command_line, conflicts_with = "words")]
    #[getset(get = "pub")]
    command: Option<CommandLine>,
    /// A new working directory, read from the current folder.
    #[arg(long, value_name = "DIR", conflicts_with = "no_cwd")]
    #[getset(get = "pub")]
    cwd: Option<PathBuf>,
    /// Run in the project's folder again.
    #[arg(long)]
    #[getset(get_copy = "pub")]
    no_cwd: bool,
    /// A new one-line description.
    #[arg(long, value_name = "TEXT", conflicts_with = "no_description")]
    #[getset(get = "pub")]
    description: Option<String>,
    /// Remove the description.
    #[arg(long)]
    #[getset(get_copy = "pub")]
    no_description: bool,
    /// Remove an environment variable (repeatable); one that is not set is
    /// left alone.
    #[arg(long = "unset-env", value_name = "KEY")]
    #[getset(get = "pub")]
    unset_env: Vec<String>,
    /// Remove the readiness check.
    #[arg(long, conflicts_with = READY_GROUP)]
    #[getset(get_copy = "pub")]
    no_ready: bool,
    #[command(flatten)]
    #[getset(get = "pub")]
    flags: ProcessFlags,
    /// Print the result as JSON.
    #[arg(long)]
    #[getset(get_copy = "pub")]
    json: bool,
    /// A new command, after `--`, joined as `muster add` joins it.
    #[arg(last = true, value_name = "COMMAND")]
    #[getset(get = "pub")]
    words: Vec<String>,
}
