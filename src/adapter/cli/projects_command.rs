use clap::Subcommand;

/// Project actions under `muster projects`. Absent, projects are listed.
#[derive(Subcommand)]
pub enum ProjectsCommand {
    /// Stop a project's processes and forget it with its processes and
    /// coordination state; its folder is untouched.
    Rm {
        /// The project's name, or its folder.
        project: String,
        /// Remove without asking; required where nobody can be asked.
        #[arg(short, long)]
        yes: bool,
    },
}
