use crate::domain::value::ProcessRef;

/// The process a lifecycle command acts on.
#[derive(clap::Args)]
pub struct ProcessArgs {
    /// The process, as `muster ps` lists it; quote a name with spaces.
    /// Chosen from a list at a terminal when absent.
    pub name: Option<ProcessRef>,
    /// Print the result as JSON.
    #[arg(long)]
    pub json: bool,
}
