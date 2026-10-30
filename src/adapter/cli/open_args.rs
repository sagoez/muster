use getset::{CopyGetters, Getters};

use super::args::split_direction_parser;
use crate::domain::{terminal_workspace::SplitDirection, value::ProcessRef};

/// The process `muster open` shows, and how.
#[derive(clap::Args, Getters, CopyGetters)]
pub struct OpenArgs {
    /// The process, as `muster ps` lists it; quote a name with spaces.
    /// Chosen from a list at a terminal when absent.
    #[getset(get = "pub")]
    name: Option<ProcessRef>,
    /// Watch without typing into the process or resizing it.
    #[arg(long)]
    #[getset(get_copy = "pub")]
    read_only: bool,
    /// Which side of this pane the new one opens on; without it, the pane
    /// splits along its longer side.
    #[arg(long, value_name = "DIRECTION", value_parser = split_direction_parser())]
    #[getset(get_copy = "pub")]
    direction: Option<SplitDirection>,
    /// Print the opened pane, or a refusal or failure, as JSON.
    #[arg(long)]
    #[getset(get_copy = "pub")]
    json: bool,
}
