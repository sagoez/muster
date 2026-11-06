//! muster's use cases: stack editing, stack sync, the workspace beside the
//! caller and process lifecycle, each reaching the world only through the
//! domain's ports.

#[cfg(test)]
mod editor_fakes;
#[cfg(test)]
mod editor_fixtures;
#[cfg(test)]
mod editor_step;
mod exit_decision;
mod process_lifecycle;
mod stack_editor;
mod stack_sync;
mod workspace;

pub use exit_decision::*;
pub use process_lifecycle::*;
pub use stack_editor::*;
pub use stack_sync::*;
pub use workspace::*;
