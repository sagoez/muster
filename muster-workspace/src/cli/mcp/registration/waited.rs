use std::process::ExitStatus;

/// How waiting on the agent's command ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waited {
    /// The command ended with this status.
    Ended(ExitStatus),
    /// A second stopping signal, this one last, ended the wait at once.
    Repeated(i32),
}
