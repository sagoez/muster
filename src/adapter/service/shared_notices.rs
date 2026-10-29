use std::sync::{Arc, Mutex};

use super::notices::Notices;

/// Notices shared between the supervisors that raise them and the readers that
/// show them.
pub type SharedNotices = Arc<Mutex<Notices>>;
