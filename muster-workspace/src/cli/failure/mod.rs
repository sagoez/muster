//! `CliError`, and the errors that embed or wrap a source.

#[cfg(test)]
pub(crate) mod embedding;
pub(crate) mod error;
#[cfg(test)]
pub(crate) mod wrapped;

pub use error::*;
