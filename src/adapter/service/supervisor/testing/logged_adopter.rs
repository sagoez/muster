use crate::{
    adapter::{holder::HolderRecord, service::supervisor::HolderAdopter},
    domain::{
        port::{OutputSink, ProcessHandle},
        pty::PtyError,
        value::{OutputCursor, Pid, ProcessName, ProjectRoot},
    },
};

/// An adopter with no running holders whose processes' logs all end with
/// `tail`, which starts at `at`, as the logs of an earlier service's runs do.
pub(crate) struct LoggedAdopter {
    /// Where the tail starts in the log.
    at: OutputCursor,
    /// The last bytes of every log.
    tail: Vec<u8>,
}

impl LoggedAdopter {
    /// An adopter whose logs end with `tail`, starting at `at`.
    pub(crate) fn new(at: OutputCursor, tail: &[u8]) -> Self {
        Self {
            at,
            tail: tail.to_vec(),
        }
    }
}

impl HolderAdopter for LoggedAdopter {
    fn locate(&self, _child_pid: Pid) -> Option<HolderRecord> {
        None
    }

    fn serving(&self, _child_pid: Pid) -> Option<HolderRecord> {
        None
    }

    fn adopt(
        &self,
        _record: &HolderRecord,
        _since: Option<OutputCursor>,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Err(PtyError::System(String::new()))
    }

    fn log_tail(
        &self,
        _project: &ProjectRoot,
        _name: &ProcessName,
    ) -> Option<(OutputCursor, Vec<u8>)> {
        Some((self.at, self.tail.clone()))
    }
}
