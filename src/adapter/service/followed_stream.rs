use std::{
    fs::File,
    io::{self, Stderr},
    os::fd::AsFd,
};

use rustix::stdio::dup2_stderr;

/// An open stream a log rotation keeps writing to the live log: when it
/// writes to the file being rotated out, it is pointed at the fresh one.
pub trait FollowedStream: Send {
    /// A second handle on the file the stream writes to, to compare with the
    /// log.
    ///
    /// # Errors
    /// Returns the I/O error from duplicating the stream.
    fn target(&self) -> io::Result<File>;

    /// Points the stream at `fresh`.
    ///
    /// # Errors
    /// Returns the I/O error from redirecting the stream.
    fn repoint(&mut self, fresh: &File) -> io::Result<()>;
}

impl FollowedStream for Stderr {
    fn target(&self) -> io::Result<File> {
        self.as_fd().try_clone_to_owned().map(File::from)
    }

    fn repoint(&mut self, fresh: &File) -> io::Result<()> {
        Ok(dup2_stderr(fresh)?)
    }
}
