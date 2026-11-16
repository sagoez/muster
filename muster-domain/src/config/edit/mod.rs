//! Edits to a stack: drafts, changes, the fields they name, the edit log,
//! and where a shared write registers its project.

mod applied_change;
mod field_edit;
mod mcp_stack_edits;
mod process_change;
mod process_draft;
mod process_field;
mod recorded_edit;
mod registering;
mod registration;
mod stack_edit;
mod write_place;
mod written;

pub use applied_change::*;
pub use field_edit::*;
pub use mcp_stack_edits::*;
pub use process_change::*;
pub use process_draft::*;
pub use process_field::*;
pub use recorded_edit::*;
pub use registering::*;
pub use registration::*;
pub use stack_edit::*;
pub use write_place::*;
pub use written::*;
