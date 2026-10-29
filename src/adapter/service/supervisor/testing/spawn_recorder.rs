use std::{cell::RefCell, rc::Rc};

use crate::domain::pty::SpawnRequest;

/// Shared record of complete PTY requests for command and environment
/// assertions.
#[derive(Clone, Default)]
pub(crate) struct SpawnRecorder {
    pub(crate) requests: Rc<RefCell<Vec<SpawnRequest>>>,
}
