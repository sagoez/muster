use std::io::{Read, Write};

use super::{
    client_role::ClientRole,
    error::HolderError,
    frame::{Frame, read_frame, write_frame},
    frame_tag::FrameTag,
    holder_control::HolderControl,
    protocol::{decode_size, encode_size},
};
use crate::domain::pty::PtySize;

/// A message from a client to the holder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMessage {
    /// Introduces the client; must come first.
    Hello {
        /// How the client participates.
        role: ClientRole,
    },
    /// Bytes to write to the child.
    Input(Vec<u8>),
    /// The client's terminal size.
    Resize(PtySize),
    /// A process control request.
    Control(HolderControl),
    /// Acknowledges the held exit outcome, letting the holder finish.
    CollectExit,
}

impl ClientMessage {
    /// Writes this message as one frame.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when encoding or the write fails.
    pub fn write_to(self, writer: &mut impl Write) -> Result<(), HolderError> {
        Ok(write_frame(writer, &Frame::try_from(self)?)?)
    }

    /// Reads one client message.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when the read fails or the frame is not a
    /// well-formed client message.
    pub fn read_from(reader: &mut impl Read) -> Result<Self, HolderError> {
        Self::try_from(read_frame(reader)?)
    }
}

impl TryFrom<ClientMessage> for Frame {
    type Error = HolderError;

    /// Encodes a client message into its frame.
    fn try_from(message: ClientMessage) -> Result<Self, Self::Error> {
        let (tag, payload) = match message {
            ClientMessage::Hello { role } => (FrameTag::Hello, serde_json::to_vec(&role)?),
            ClientMessage::Input(bytes) => (FrameTag::Input, bytes),
            ClientMessage::Resize(size) => (FrameTag::Resize, encode_size(size).to_vec()),
            ClientMessage::Control(control) => (FrameTag::Control, serde_json::to_vec(&control)?),
            ClientMessage::CollectExit => (FrameTag::CollectExit, Vec::new()),
        };
        Ok(Frame::builder().tag(tag).payload(payload).build())
    }
}

impl TryFrom<Frame> for ClientMessage {
    type Error = HolderError;

    /// Decodes a frame into a client message.
    fn try_from(frame: Frame) -> Result<Self, Self::Error> {
        let tag = frame.tag();
        let payload = frame.into_payload();
        match tag {
            FrameTag::Hello => Ok(Self::Hello {
                role: serde_json::from_slice(&payload)?,
            }),
            FrameTag::Input => Ok(Self::Input(payload)),
            FrameTag::Resize => decode_size(&payload)
                .map(Self::Resize)
                .ok_or(HolderError::MalformedPayload(tag)),
            FrameTag::Control => Ok(Self::Control(serde_json::from_slice(&payload)?)),
            FrameTag::CollectExit if payload.is_empty() => Ok(Self::CollectExit),
            FrameTag::CollectExit => Err(HolderError::MalformedPayload(tag)),
            FrameTag::Snapshot
            | FrameTag::Output
            | FrameTag::Exited
            | FrameTag::Rejected
            | FrameTag::Warning
            | FrameTag::Attached => Err(HolderError::UnexpectedFrame(tag)),
        }
    }
}
