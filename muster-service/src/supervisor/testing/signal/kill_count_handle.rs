use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use getset::Getters;
use muster_domain::pty::PtyError;
use typed_builder::TypedBuilder;

use crate::supervisor::testing::StubHandle;

/// A handle that counts how many times it is killed.
#[derive(Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct KillCountHandle {
    kills: Arc<AtomicUsize>,
}

impl StubHandle for KillCountHandle {
    fn kill(&mut self) -> Result<(), PtyError> {
        self.kills.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
