use getset::{CopyGetters, Getters};

use crate::domain::value::ProcessRef;

/// The process a lifecycle command acts on.
#[derive(clap::Args, Getters, CopyGetters)]
pub struct ProcessArgs {
    /// The process, as `muster ps` lists it; quote a name with spaces.
    /// Chosen from a list at a terminal when absent.
    #[getset(get = "pub")]
    name: Option<ProcessRef>,
    /// Print the result as JSON.
    #[arg(long)]
    #[getset(get_copy = "pub")]
    json: bool,
}
