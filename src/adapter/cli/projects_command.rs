use clap::Subcommand;

use super::idle_stop_override::{IdleStopOverride, idle_stop_override};

/// The id of the project a `projects` action names, apart from the global
/// `--project`'s, which clap would otherwise neither show nor keep apart.
const PROJECT_OPERAND: &str = "project-operand";
/// How help names that project.
const PROJECT_VALUE: &str = "PROJECT";

/// Project actions under `muster projects`. Absent, projects are listed.
#[derive(Subcommand)]
pub enum ProjectsCommand {
    /// Stop a project's processes and forget it with its processes and
    /// coordination state; its folder is untouched.
    Rm {
        /// The project's name, or its folder.
        #[arg(id = PROJECT_OPERAND, value_name = PROJECT_VALUE)]
        project: String,
        /// Remove without asking; required where nobody can be asked.
        #[arg(short, long)]
        yes: bool,
    },
    /// Change a project's own settings.
    Set {
        /// The project's name, or its folder; the current project when
        /// absent.
        #[arg(id = PROJECT_OPERAND, value_name = PROJECT_VALUE)]
        project: Option<String>,
        /// How long the project keeps running once no shell is in it: a
        /// duration such as 30m, never, or default to follow the idle-stop
        /// setting.
        #[arg(long, value_name = "DURATION", value_parser = idle_stop_override)]
        idle_stop: IdleStopOverride,
    },
}
