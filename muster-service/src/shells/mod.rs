//! Hooked shells: their notes, the sweep for gone ones, and what entering a
//! project starts.

pub(crate) mod entering_launches;
#[cfg(test)]
pub(crate) mod fake_shells;
pub(crate) mod shell_notes;
pub(crate) mod shell_sweeper;
pub(crate) mod stopping_entries;

pub use entering_launches::*;
pub use shell_notes::*;
pub use shell_sweeper::*;
pub use stopping_entries::*;
