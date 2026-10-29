use std::path::PathBuf;

use bytesize::ByteSize as RawByteSize;
use getset::{Getters, MutGetters};
use tempfile::TempDir;

use super::{log::RotatingLog, log_query::LogQuery, log_reader::read_log, log_slice::LogSlice};
use crate::domain::value::{ByteSize, OutputCursor};

/// A log in a scratch directory, written through the holder's own log.
#[derive(Getters, MutGetters)]
pub(super) struct Fixture {
    _scratch: TempDir,
    #[getset(get = "pub(super)", get_mut = "pub(super)")]
    path: PathBuf,
    #[getset(get = "pub(super)", get_mut = "pub(super)")]
    log: RotatingLog,
}

impl Fixture {
    /// A fresh log rotating at `cap` bytes.
    pub(super) fn with_cap(cap: u64) -> Self {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join("api.log");
        let log = RotatingLog::open(
            path.clone(),
            ByteSize::try_new(RawByteSize::b(cap)).unwrap(),
        )
        .unwrap();
        Self {
            _scratch: scratch,
            path,
            log,
        }
    }

    /// A log that never rotates in these tests.
    pub(super) fn new() -> Self {
        Self::with_cap(1 << 30)
    }

    /// Appends `bytes` to the log and returns where they start.
    pub(super) fn write(&mut self, bytes: &[u8]) -> OutputCursor {
        let at = self.log.end();
        self.log.append(bytes).unwrap();
        at
    }

    /// Reads the log as `query` asks.
    pub(super) fn read(&self, query: &LogQuery) -> LogSlice {
        read_log(&self.path, query).unwrap()
    }
}
