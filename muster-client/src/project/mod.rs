//! Finding the project a command or tool call acts on, and naming it, and
//! its processes, in a command to paste.

mod command_words;
#[cfg(any(test, feature = "testing"))]
pub mod fixed_projects;
mod project_target;
mod refused_folder;
mod resolve_error;
mod working_dir_error;
mod working_dir_place;

pub use command_words::*;
pub use project_target::*;
pub use refused_folder::*;
pub use resolve_error::*;
pub use working_dir_error::*;
pub use working_dir_place::*;
