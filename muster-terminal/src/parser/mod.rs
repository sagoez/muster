//! The bounded vte parser and the escape-sequence vocabulary every scanner
//! shares.

pub mod bounded_parser;
#[cfg(test)]
mod dispatch_recorder;
pub mod escape;
pub mod fed;
mod watched_performer;
