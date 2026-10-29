use std::{
    sync::{Arc, Mutex},
    thread::JoinHandle,
};

use crate::adapter::holder::HolderError;

/// Every holder thread a fixture's runners started.
pub(super) type Holders = Arc<Mutex<Vec<JoinHandle<Result<(), HolderError>>>>>;
