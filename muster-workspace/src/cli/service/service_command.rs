use clap::Subcommand;

/// Service actions under `muster service`.
#[derive(Subcommand)]
pub enum ServiceCommand {
    /// Show whether the service runs, its version and what it has loaded,
    /// without starting it.
    Status {
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Stop the service. Every process keeps running under its holder, and
    /// the next muster command starts the service again.
    Stop {
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
}
