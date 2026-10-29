/// Configured result of graceful termination in retry tests.
#[derive(Clone, Copy)]
pub(crate) enum TerminateOutcome {
    Succeeds,
    Fails,
}
