use crate::domain::port::ProcessHandle;

/// How the process of a launch or adoption result that came back stale is
/// let go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaleRelease {
    /// A shutdown superseded the launch: the handle is dropped unkilled, so
    /// its holder keeps the process for the next service.
    Detach,
    /// A user's action superseded the launch, or nothing records it: the
    /// process is stopped so nothing is left running unsupervised.
    Discard,
}

impl StaleRelease {
    /// Lets go of `handle` as this release says.
    pub fn release(self, handle: Box<dyn ProcessHandle>) {
        match self {
            Self::Detach => drop(handle),
            Self::Discard => handle.abandon(),
        }
    }
}
