use std::path::PathBuf;

use bytesize::ByteSize as RawByteSize;
use tempfile::TempDir;

use super::{log::RotatingLog, log_query::LogQuery, log_reader::read_log, log_slice::LogSlice};
use crate::domain::value::{ByteSize, OutputCursor};

/// A log in a scratch directory, written through the holder's own log.
pub(super) struct Fixture {
    _scratch: TempDir,
    pub(super) path: PathBuf,
    pub(super) log: RotatingLog,
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

    pub(super) fn write(&mut self, bytes: &[u8]) -> OutputCursor {
        let at = self.log.end();
        self.log.append(bytes).unwrap();
        at
    }

    pub(super) fn read(&self, query: &LogQuery) -> LogSlice {
        read_log(&self.path, query).unwrap()
    }
}
