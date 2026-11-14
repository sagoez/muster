//! Port lookups and socket-table scans.

mod asked_scans;
mod occupied;

pub(crate) use asked_scans::*;
pub(crate) use occupied::*;
