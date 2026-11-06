use std::sync::{Arc, Mutex};

use muster_domain::port::PortOccupants;

/// A lookup of who listens on a port, shared by a supervisor and the launch
/// workers that ask it off the loop.
pub(crate) type SharedPortOccupants = Arc<Mutex<Box<dyn PortOccupants>>>;
