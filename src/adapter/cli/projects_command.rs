use clap::Subcommand;

use super::idle_stop_override::{IdleStopOverride, idle_stop_override};

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
    /// Change a project's own settings.
    Set {
        /// The project's name, or its folder; the current project when
        /// absent.
        project: Option<String>,
        /// How long the project keeps running once no shell is in it: a
        /// duration such as 30m, never, or default to follow the idle-stop
        /// setting.
        #[arg(long, value_name = "DURATION", value_parser = idle_stop_override)]
        idle_stop: IdleStopOverride,
    },
}
