//! Notices, and the service log they are mirrored to.

mod followed_stream;
mod log_appender;
mod mirror_write;
pub(crate) mod notice_mirror;
pub(crate) mod notices;
#[cfg(test)]
mod shared_fd;
pub(crate) mod shared_notices;

pub use notices::*;
pub use shared_notices::*;
