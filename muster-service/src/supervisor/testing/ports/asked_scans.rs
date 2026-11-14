use std::sync::{Arc, Mutex};

use muster_domain::value::LocalPort;

/// The ports each socket-table scan was asked about, in order.
pub(crate) type AskedScans = Arc<Mutex<Vec<Vec<LocalPort>>>>;
