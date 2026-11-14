//! The command line itself: parsed arguments and subcommands, the ports the
//! composition root hands the CLI, completions and exit codes.

pub(crate) mod args;
pub(crate) mod cli_exit;
pub(crate) mod cli_ports;
pub(crate) mod command;
pub(crate) mod completions;
#[cfg(test)]
pub(crate) mod fake_ports;
pub(crate) mod invocation;

pub use args::*;
pub use cli_exit::*;
pub use cli_ports::*;
pub use command::*;
pub use completions::*;
