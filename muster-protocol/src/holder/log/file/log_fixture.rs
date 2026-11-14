use std::path::PathBuf;

use bytesize::ByteSize as RawByteSize;
use getset::{Getters, MutGetters};
use muster_domain::value::{ByteSize, OutputCursor};
use tempfile::TempDir;

use crate::holder::log::{
    file::log::RotatingLog,
    read::{log_query::LogQuery, log_reader::read_log, log_slice::LogSlice},
};

/// A log in a scratch directory, written through the holder's own log.
#[derive(Getters, MutGetters)]
pub struct Fixture {
    _scratch: TempDir,
    #[getset(get = "pub", get_mut = "pub")]
    path: PathBuf,
    #[getset(get = "pub", get_mut = "pub")]
    log: RotatingLog,
}

impl Fixture {
    /// A fresh log rotating at `cap` bytes.
    pub fn with_cap(cap: u64) -> Self {
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

    /// Appends `bytes` to the log and returns where they start.
    pub fn write(&mut self, bytes: &[u8]) -> OutputCursor {
        let at = self.log.end();
        self.log.append(bytes).unwrap();
        at
    }

    /// Reads the log as `query` asks.
    pub fn read(&self, query: &LogQuery) -> LogSlice {
        read_log(&self.path, query).unwrap()
    }
}

impl Default for Fixture {
    /// A log that never rotates in these tests.
    fn default() -> Self {
        Self::with_cap(1 << 30)
    }
}
