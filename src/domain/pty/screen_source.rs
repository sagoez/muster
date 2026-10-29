use std::sync::Arc;

use crate::domain::port::ScreenReader;

/// Where the authoritative screen of a running process is read.
#[derive(Debug, Clone)]
pub enum ScreenSource {
    /// The last screen the handle repainted, which the supervisor keeps.
    Kept,
    /// A separate endpoint serves the screen, read through this reader.
    Served(Arc<dyn ScreenReader>),
}
