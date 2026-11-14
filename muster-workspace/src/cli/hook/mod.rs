//! The shell hook's commands: `shell-init`, the report each prompt runs,
//! and the notes it prints.

pub(crate) mod hooked_shell;
pub(crate) mod shell_init;
mod shell_note_text;
pub(crate) mod shell_registration;
pub(crate) mod shell_report;
mod shell_report_skip;

pub use hooked_shell::hooked_shell_in;
