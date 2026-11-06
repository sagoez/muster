use std::sync::Arc;

use muster_domain::{port::ProcessHandle, pty::PtyError};

use super::{adopter::HolderAdopter, channel_output_sink::ChannelOutputSink};

/// How an adoption proceeds: inline, its result already in hand, or on a
/// worker with the adopter and sink it takes there.
pub(super) enum Adoption {
    /// The adoption ran on the caller's thread.
    Inline(Result<Box<dyn ProcessHandle>, PtyError>),
    /// The adoption runs on a worker.
    Detached(Arc<dyn HolderAdopter + Send + Sync>, ChannelOutputSink),
}
