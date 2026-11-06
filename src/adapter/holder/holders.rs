use std::{
    sync::{Arc, Mutex},
    thread::JoinHandle,
};

use muster_protocol::holder::HolderError;

/// Every holder thread a fixture's runners started.
pub(crate) type Holders = Arc<Mutex<Vec<JoinHandle<Result<(), HolderError>>>>>;
