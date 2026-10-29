use super::{args::positive_duration, process_args::ProcessArgs};
use crate::domain::value::PositiveDuration;

/// The process a start or restart acts on, and whether to wait for it.
#[derive(clap::Args)]
pub struct StartArgs {
    /// The process and output form.
    #[command(flatten)]
    pub target: ProcessArgs,
    /// Wait until its new run passes its readiness check, or runs when it has
    /// none.
    #[arg(long)]
    pub wait: bool,
    /// With --wait, the longest to wait, such as 60s; unbounded when absent.
    #[arg(long, requires = "wait", value_name = "DURATION", value_parser = positive_duration)]
    pub timeout: Option<PositiveDuration>,
}
