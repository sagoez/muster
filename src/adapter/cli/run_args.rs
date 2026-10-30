use getset::{CopyGetters, Getters};

/// `muster run`: a command to add, start and attach to.
#[derive(clap::Args, Getters, CopyGetters)]
pub struct RunArgs {
    /// Its name. Defaults to the command's first word.
    #[arg(short, long)]
    #[getset(get = "pub")]
    name: Option<String>,
    /// Print the report as JSON.
    #[arg(long)]
    #[getset(get_copy = "pub")]
    json: bool,
    /// The command, e.g. `-- npm run dev`.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
    #[getset(get = "pub")]
    command: Vec<String>,
}
