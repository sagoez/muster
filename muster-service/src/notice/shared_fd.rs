use std::{
    fs::File,
    io::{self, Write},
    os::fd::OwnedFd,
    sync::{Arc, Mutex},
};

use muster_os::locked::Locked;
use rustix::io::dup2;

use super::followed_stream::FollowedStream;

/// A file descriptor a test and a log rotation both hold, standing in for
/// the service's stderr.
#[derive(Clone)]
pub struct SharedFd(Arc<Mutex<OwnedFd>>);

impl SharedFd {
    /// Shares `file`'s descriptor.
    pub fn new(file: File) -> Self {
        Self(Arc::new(Mutex::new(OwnedFd::from(file))))
    }

    /// Writes `bytes` to whatever the descriptor now points at.
    pub fn write(&self, bytes: &[u8]) {
        let fd = self.0.locked();
        File::from(fd.try_clone().unwrap())
            .write_all(bytes)
            .unwrap();
    }
}

impl FollowedStream for SharedFd {
    fn target(&self) -> io::Result<File> {
        let fd = self.0.locked();
        fd.try_clone().map(File::from)
    }

    fn repoint(&mut self, fresh: &File) -> io::Result<()> {
        let mut fd = self.0.locked();
        Ok(dup2(fresh, &mut fd)?)
    }
}
