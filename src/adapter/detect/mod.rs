//! What `muster init` offers to add: candidate processes read from the files
//! a folder already has. Files are only read; nothing is ever run.

mod candidate;
mod compose;
mod detect_source;
mod detection;
mod justfile;
mod makefile;
mod package_json;
mod procfile;
mod source_file;

pub use candidate::*;
pub use detect_source::*;
pub use detection::*;
