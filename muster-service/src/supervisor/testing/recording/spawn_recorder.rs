use std::{cell::RefCell, rc::Rc};

use getset::Getters;
use muster_domain::pty::SpawnRequest;
use typed_builder::TypedBuilder;

/// Shared record of complete PTY requests for command and environment
/// assertions.
#[derive(Clone, Default, Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct SpawnRecorder {
    requests: Rc<RefCell<Vec<SpawnRequest>>>,
}
