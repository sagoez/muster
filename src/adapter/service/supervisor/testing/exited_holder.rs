use std::{os::unix::net::UnixListener, path::PathBuf, thread};

use crossbeam_channel::{Receiver, bounded};
use getset::Getters;
use tempfile::TempDir;
use typed_builder::TypedBuilder;

use super::fixtures::SETTLE_WAIT;
use crate::{
    adapter::{
        holder::{ClientMessage, HolderControl, HolderMessage},
        test_scratch::short_scratch,
    },
    domain::pty::ExitOutcome,
};

/// Prefix of every exited holder's scratch directory.
const SCRATCH_PREFIX: &str = "ma-";

/// A fake holder at a socket in a scratch directory that takes one
/// supervisor, tells it the child exited, and reports whether the exit
/// was collected.
#[derive(Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct ExitedHolder {
    _scratch: TempDir,
    socket: PathBuf,
    collected: Receiver<()>,
}

impl ExitedHolder {
    /// A holder serving in a fresh scratch directory until its exit is
    /// collected.
    pub(crate) fn serve() -> Self {
        let scratch = short_scratch(SCRATCH_PREFIX);
        let socket = scratch.path().join("h.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let (sender, collected) = bounded(1);
        thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut exit_sent = false;
            while let Ok(message) = ClientMessage::read_from(&mut stream) {
                match message {
                    ClientMessage::Control(HolderControl::Kill) | ClientMessage::Hello { .. }
                        if !exit_sent =>
                    {
                        exit_sent = true;
                        let _ = HolderMessage::Exited(ExitOutcome::Succeeded).write_to(&mut stream);
                    },
                    ClientMessage::CollectExit => {
                        let _ = sender.send(());
                        return;
                    },
                    _ => {},
                }
            }
        });
        Self {
            _scratch: scratch,
            socket,
            collected,
        }
    }

    /// Whether a supervisor collected the exit within [`SETTLE_WAIT`].
    pub(crate) fn was_collected(&self) -> bool {
        self.collected.recv_timeout(SETTLE_WAIT).is_ok()
    }
}
