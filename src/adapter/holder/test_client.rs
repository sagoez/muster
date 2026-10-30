use std::{io, os::unix::net::UnixStream, path::Path};

use getset::{Getters, MutGetters};

use super::{
    client_message::ClientMessage,
    client_role::ClientRole,
    error::HolderError,
    protocol::HolderMessage,
    server_testing::{SCROLLBACK, WAIT, wait_until},
};
use crate::{
    adapter::terminal::TerminalEmulator,
    domain::{color_scheme::ColorScheme, value::OutputCursor},
};

/// One client connection with a bounded read timeout, mirroring the screen
/// it is shown.
#[derive(Getters, MutGetters)]
pub(super) struct TestClient {
    #[getset(get = "pub(super)", get_mut = "pub(super)")]
    stream: UnixStream,
    #[getset(get = "pub(super)", get_mut = "pub(super)")]
    output: Vec<u8>,
    #[getset(get = "pub(super)", get_mut = "pub(super)")]
    frames: Vec<(OutputCursor, usize)>,
    #[getset(get = "pub(super)", get_mut = "pub(super)")]
    screen: Option<TerminalEmulator>,
    #[getset(get = "pub(super)", get_mut = "pub(super)")]
    snapshots: usize,
}

impl TestClient {
    /// Connects to the holder at `socket` as `role`, once it serves, and
    /// returns the client with the snapshot it was greeted with.
    pub(super) fn connect(socket: &Path, role: ClientRole) -> (Self, HolderMessage) {
        let mut stream = None;
        wait_until("the holder socket", || {
            stream = UnixStream::connect(socket).ok();
            stream.is_some()
        });
        let stream = stream.unwrap();
        stream.set_read_timeout(Some(WAIT)).unwrap();
        let mut client = Self {
            stream,
            output: Vec::new(),
            frames: Vec::new(),
            screen: None,
            snapshots: 0,
        };
        client.send(ClientMessage::Hello { role });
        let snapshot = client.recv();
        (client, snapshot)
    }

    /// Sends `message` to the holder.
    pub(super) fn send(&mut self, message: ClientMessage) {
        message.write_to(&mut self.stream).unwrap();
    }

    /// Receives the next message, keeping the output, frames, screen and
    /// snapshot count it carries.
    pub(super) fn recv(&mut self) -> HolderMessage {
        let message = HolderMessage::read_from(&mut self.stream).unwrap();
        match &message {
            HolderMessage::Output { at, bytes } => {
                self.frames.push((*at, bytes.len()));
                self.output.extend_from_slice(bytes);
                if let Some(screen) = self.screen.as_mut() {
                    screen.process(bytes);
                }
            },
            HolderMessage::Snapshot { repaint, size, .. } => {
                let mut screen =
                    TerminalEmulator::new(size.rows(), size.cols(), SCROLLBACK, ColorScheme::Dark);
                screen.process(repaint);
                self.screen = Some(screen);
                self.snapshots += 1;
            },
            HolderMessage::Exited(_)
            | HolderMessage::Rejected(_)
            | HolderMessage::Warning(_)
            | HolderMessage::Attached(_) => {},
        }
        message
    }

    /// Reads until the mirrored screen shows `needle`, returning its text.
    pub(super) fn screen_until(&mut self, needle: &str) -> String {
        loop {
            let text = self
                .screen
                .as_ref()
                .map(TerminalEmulator::screen_text)
                .unwrap_or_default();
            if text.contains(needle) {
                return text;
            }
            self.recv();
        }
    }

    /// Reads until the holder closes the connection, failing on a timeout.
    pub(super) fn expect_closed(&mut self) {
        loop {
            match HolderMessage::read_from(&mut self.stream) {
                Ok(_) => {},
                Err(HolderError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof => {
                    return;
                },
                Err(error) => panic!("expected the holder to close the connection: {error}"),
            }
        }
    }

    /// Receives until a message `wanted` accepts, and returns it.
    pub(super) fn recv_until(
        &mut self,
        mut wanted: impl FnMut(&HolderMessage) -> bool,
    ) -> HolderMessage {
        loop {
            let message = self.recv();
            if wanted(&message) {
                return message;
            }
        }
    }

    /// Receives until the output holds `needle`, and returns the output.
    pub(super) fn output_until(&mut self, needle: &str) -> String {
        while !String::from_utf8_lossy(&self.output).contains(needle) {
            self.recv();
        }
        String::from_utf8_lossy(&self.output).into_owned()
    }
}
