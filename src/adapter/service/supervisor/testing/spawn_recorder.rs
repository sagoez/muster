use std::{cell::RefCell, rc::Rc};

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::pty::SpawnRequest;

/// Shared record of complete PTY requests for command and environment
/// assertions.
#[derive(Clone, Default, Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct SpawnRecorder {
    requests: Rc<RefCell<Vec<SpawnRequest>>>,
}
