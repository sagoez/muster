//! Which lines of a log a read keeps: patterns, limits and errors-only.

pub(crate) mod byte_limit;
pub(crate) mod error_line;
pub(crate) mod filter_state;
pub(crate) mod line_filter;
pub(crate) mod line_limit;
pub(crate) mod line_matcher;
mod matcher;
mod pattern_error;
pub(crate) mod strip;

pub use byte_limit::*;
pub use line_filter::*;
pub use line_limit::*;
pub use line_matcher::*;
pub use pattern_error::*;
pub use strip::*;
