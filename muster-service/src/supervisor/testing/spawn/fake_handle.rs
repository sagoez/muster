use crate::supervisor::testing::StubHandle;

/// A handle every operation of which succeeds and does nothing.
pub(crate) struct FakeHandle;

impl StubHandle for FakeHandle {}
