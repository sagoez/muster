//! The terminal workspace port over the herdr CLI: each call runs the
//! installed `herdr` binary as a short, bounded child and reads its JSON.

#[cfg(test)]
mod fake_herdr;
mod herdr_call;
mod herdr_failure;
mod herdr_workspace;
mod pane_identity;
mod reply;
mod typed_line;

pub use herdr_workspace::*;
pub use pane_identity::*;
