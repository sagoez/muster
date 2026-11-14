//! Editing the stored stack: `add`, `set`, `rm`, `run` and `init`.

pub(crate) mod add;
pub(crate) mod add_args;
mod command_words;
pub(crate) mod init;
mod process_flags;
pub(crate) mod rm;
pub(crate) mod run;
pub(crate) mod run_args;
pub(crate) mod set;
pub(crate) mod set_args;

pub use add_args::*;
pub use command_words::*;
pub use run_args::*;
pub use set_args::*;
