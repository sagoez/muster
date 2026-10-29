/// `muster run`: a command to add, start and attach to.
#[derive(clap::Args)]
pub struct RunArgs {
    /// Its name. Defaults to the command's first word.
    #[arg(short, long)]
    pub name: Option<String>,
    /// Print the report as JSON.
    #[arg(long)]
    pub json: bool,
    /// The command, e.g. `-- npm run dev`.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
    pub command: Vec<String>,
}
