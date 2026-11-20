use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use getset::Getters;
use muster_domain::pty::PtyError;
use typed_builder::TypedBuilder;

use crate::supervisor::testing::StubHandle;

/// A handle that counts the exits acknowledged to it.
#[derive(Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct AckCountHandle {
    acks: Arc<AtomicUsize>,
}

impl StubHandle for AckCountHandle {
    fn acknowledge_exit(&mut self) -> Result<(), PtyError> {
        self.acks.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
