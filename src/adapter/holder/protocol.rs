//! The messages a holder and its clients exchange, each carried in one
//! [`Frame`]. Byte streams travel raw; structured bodies are JSON.

use std::{
    io::{Read, Write},
    time::Duration,
};

use serde::{Deserialize, Serialize};

use super::{
    error::HolderError,
    frame::{Frame, FrameTag, read_frame, write_frame},
};
use crate::{
    adapter::wire::WireForm,
    domain::{
        process::StopSignal,
        pty::{ExitOutcome, PtySize},
        value::{OutputCursor, Pid},
    },
};

/// Bytes an encoded [`PtySize`] occupies: big-endian rows then cols.
const SIZE_BYTES: usize = 4;
/// Bytes an encoded child pid occupies.
const PID_BYTES: usize = 4;
/// Bytes an encoded attach-client count occupies.
const ATTACHED_BYTES: usize = 4;
/// Bytes an encoded output offset occupies, ahead of the output it places.
const OFFSET_BYTES: usize = 8;
/// Bytes of a snapshot payload before its repaint: the pid, the size, the
/// attach-client count, then the run's start offset.
const SNAPSHOT_HEADER_BYTES: usize = PID_BYTES + SIZE_BYTES + ATTACHED_BYTES + OFFSET_BYTES;

/// How a connected client participates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", rename_all_fields = "kebab-case")]
pub enum ClientRole {
    /// The service: writes, resizes, controls the child and collects its exit.
    Supervisor,
    /// A user terminal; a read-only one only watches.
    Attach {
        /// Whether input and resizes from this client are refused.
        read_only: bool,
    },
}

impl ClientRole {
    /// Whether this client may write input and resize the PTY.
    pub fn writable(self) -> bool {
        !matches!(self, Self::Attach { read_only: true })
    }
}

/// A process control request, honoured only from a supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", rename_all_fields = "kebab-case")]
pub enum HolderControl {
    /// Suspends the child's process group.
    Pause,
    /// Resumes a suspended child's process group.
    Resume,
    /// Sends `signal`, allowing `grace` before the group is force-killed.
    Terminate {
        /// The graceful signal to send.
        #[serde(with = "crate::adapter::wire::form")]
        signal: StopSignal,
        /// Time allowed for a graceful exit.
        grace: Duration,
    },
    /// Force-kills the child's process group.
    Kill,
}

/// Why the holder refused a client message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RejectReason {
    /// The client sent a message before introducing itself.
    HelloRequired,
    /// The client sent a second hello; its role is fixed by the first.
    AlreadyIntroduced,
    /// A read-only client tried to write or resize.
    ReadOnly,
    /// A non-supervisor tried to control the child or collect its exit.
    SupervisorOnly,
    /// An exit was collected before the child exited.
    NotExited,
    /// The child refused the operation, typically because it has exited.
    ChildUnavailable,
    /// The child is not reading its input fast enough; the input was dropped.
    InputBacklogged,
}

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

/// A message from the holder to a client.
///
/// A [`Self::Snapshot`] can also arrive mid-stream as a resync: when a client
/// falls a full outbox behind, the holder discards its queued messages and
/// sends a fresh snapshot, then the held exit if any, so the client replaces
/// its screen rather than applying output on top of it. Every attach client
/// is resynced the same way when the PTY changes size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HolderMessage {
    /// The current screen, sent in answer to a hello and again as a resync.
    Snapshot {
        /// Bytes that rebuild the live screen and modes in a fresh terminal.
        repaint: Vec<u8>,
        /// The PTY size the screen was rendered at.
        size: PtySize,
        /// The child's process id.
        pid: Pid,
        /// The writable attach clients connected.
        attached: u32,
        /// Where the child's run began in the process's output stream, so a
        /// client that connects late still knows the run's first offset.
        run_start: OutputCursor,
    },
    /// Bytes the child wrote, with the output offset of the first, so a
    /// client that missed frames sees the gap.
    Output {
        /// Where `bytes` start in the process's output stream.
        at: OutputCursor,
        /// The bytes.
        bytes: Vec<u8>,
    },
    /// The child exited; repeated to each client until a supervisor collects it.
    Exited(ExitOutcome),
    /// A client message was refused.
    Rejected(RejectReason),
    /// A fault the holder recovered from, such as a screen it had to reset.
    Warning(String),
    /// The writable attach clients connected, sent to the supervisor when
    /// the count changes.
    Attached(u32),
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

impl HolderMessage {
    /// Writes this message as one frame.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when encoding or the write fails.
    pub fn write_to(self, writer: &mut impl Write) -> Result<(), HolderError> {
        Ok(write_frame(writer, &Frame::try_from(self)?)?)
    }

    /// Reads one holder message.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when the read fails or the frame is not a
    /// well-formed holder message.
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

impl TryFrom<HolderMessage> for Frame {
    type Error = HolderError;

    /// Encodes a holder message into its frame.
    fn try_from(message: HolderMessage) -> Result<Self, Self::Error> {
        let (tag, payload) = match message {
            HolderMessage::Snapshot {
                repaint,
                size,
                pid,
                attached,
                run_start,
            } => {
                let mut payload = Vec::with_capacity(SNAPSHOT_HEADER_BYTES + repaint.len());
                payload.extend_from_slice(&pid.into_inner().to_be_bytes());
                payload.extend_from_slice(&encode_size(size));
                payload.extend_from_slice(&attached.to_be_bytes());
                payload.extend_from_slice(&run_start.offset().to_be_bytes());
                payload.extend(repaint);
                (FrameTag::Snapshot, payload)
            },
            HolderMessage::Output { at, bytes } => {
                let mut payload = Vec::with_capacity(OFFSET_BYTES + bytes.len());
                payload.extend_from_slice(&at.offset().to_be_bytes());
                payload.extend(bytes);
                (FrameTag::Output, payload)
            },
            HolderMessage::Exited(outcome) => {
                (FrameTag::Exited, serde_json::to_vec(&outcome.to_wire())?)
            },
            HolderMessage::Rejected(reason) => (FrameTag::Rejected, serde_json::to_vec(&reason)?),
            HolderMessage::Warning(text) => (FrameTag::Warning, serde_json::to_vec(&text)?),
            HolderMessage::Attached(count) => (FrameTag::Attached, count.to_be_bytes().to_vec()),
        };
        Ok(Frame::builder().tag(tag).payload(payload).build())
    }
}

impl TryFrom<Frame> for HolderMessage {
    type Error = HolderError;

    /// Decodes a frame into a holder message.
    fn try_from(frame: Frame) -> Result<Self, Self::Error> {
        let tag = frame.tag();
        let mut payload = frame.into_payload();
        match tag {
            FrameTag::Snapshot => {
                if payload.len() < SNAPSHOT_HEADER_BYTES {
                    return Err(HolderError::MalformedPayload(tag));
                }
                let repaint = payload.split_off(SNAPSHOT_HEADER_BYTES);
                let (pid, rest) = payload.split_at(PID_BYTES);
                let (size, rest) = rest.split_at(SIZE_BYTES);
                let (attached, run_start) = rest.split_at(ATTACHED_BYTES);
                let pid = decode_u32(pid)
                    .and_then(|pid| Pid::try_new(pid).ok())
                    .ok_or(HolderError::MalformedPayload(tag))?;
                let size = decode_size(size).ok_or(HolderError::MalformedPayload(tag))?;
                let attached = decode_u32(attached).ok_or(HolderError::MalformedPayload(tag))?;
                let run_start =
                    decode_offset(run_start).ok_or(HolderError::MalformedPayload(tag))?;
                Ok(Self::Snapshot {
                    repaint,
                    size,
                    pid,
                    attached,
                    run_start,
                })
            },
            FrameTag::Output => {
                if payload.len() < OFFSET_BYTES {
                    return Err(HolderError::MalformedPayload(tag));
                }
                let bytes = payload.split_off(OFFSET_BYTES);
                let at = decode_offset(&payload).ok_or(HolderError::MalformedPayload(tag))?;
                Ok(Self::Output { at, bytes })
            },
            FrameTag::Exited => ExitOutcome::from_wire(serde_json::from_slice(&payload)?)
                .map(Self::Exited)
                .map_err(|_| HolderError::MalformedPayload(tag)),
            FrameTag::Rejected => Ok(Self::Rejected(serde_json::from_slice(&payload)?)),
            FrameTag::Warning => Ok(Self::Warning(serde_json::from_slice(&payload)?)),
            FrameTag::Attached => decode_u32(&payload)
                .map(Self::Attached)
                .ok_or(HolderError::MalformedPayload(tag)),
            FrameTag::Hello
            | FrameTag::Input
            | FrameTag::Resize
            | FrameTag::Control
            | FrameTag::CollectExit => Err(HolderError::UnexpectedFrame(tag)),
        }
    }
}

/// Encodes a PTY size as big-endian rows then cols.
fn encode_size(size: PtySize) -> [u8; SIZE_BYTES] {
    let [rows_high, rows_low] = size.rows().into_inner().to_be_bytes();
    let [cols_high, cols_low] = size.cols().into_inner().to_be_bytes();
    [rows_high, rows_low, cols_high, cols_low]
}

/// Decodes a big-endian `u32`, or `None` for the wrong length.
fn decode_u32(bytes: &[u8]) -> Option<u32> {
    bytes.try_into().ok().map(u32::from_be_bytes)
}

/// Decodes a big-endian output offset, or `None` for the wrong length.
fn decode_offset(bytes: &[u8]) -> Option<OutputCursor> {
    bytes
        .try_into()
        .ok()
        .map(|offset| OutputCursor::new(u64::from_be_bytes(offset)))
}

/// Decodes a PTY size written by [`encode_size`], or `None` for the wrong
/// length or a size outside the PTY bounds, which no holder renders.
fn decode_size(bytes: &[u8]) -> Option<PtySize> {
    let [rows_high, rows_low, cols_high, cols_low] = <[u8; SIZE_BYTES]>::try_from(bytes).ok()?;
    PtySize::try_new(
        u16::from_be_bytes([rows_high, rows_low]),
        u16::from_be_bytes([cols_high, cols_low]),
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::domain::pty::{ExitCause, SignalName};

    fn size(rows: u16, cols: u16) -> PtySize {
        PtySize::try_new(rows, cols).unwrap()
    }

    /// Every message survives the wire in both directions.
    #[test]
    fn messages_round_trip() {
        let client = [
            ClientMessage::Hello {
                role: ClientRole::Attach { read_only: true },
            },
            ClientMessage::Input(b"ls\r".to_vec()),
            ClientMessage::Resize(size(40, 120)),
            ClientMessage::Control(HolderControl::Terminate {
                signal: StopSignal::Interrupt,
                grace: Duration::from_millis(1500),
            }),
            ClientMessage::CollectExit,
        ];
        for message in client {
            let mut wire = Vec::new();
            message.clone().write_to(&mut wire).unwrap();
            assert_eq!(
                ClientMessage::read_from(&mut Cursor::new(wire)).unwrap(),
                message
            );
        }

        let holder = [
            HolderMessage::Snapshot {
                repaint: b"\x1b[2Jhi".to_vec(),
                size: size(24, 80),
                pid: Pid::try_new(4242).unwrap(),
                attached: 2,
                run_start: OutputCursor::new(u64::MAX - 7),
            },
            HolderMessage::Output {
                at: OutputCursor::new(u64::MAX - 3),
                bytes: b"out".to_vec(),
            },
            HolderMessage::Output {
                at: OutputCursor::new(0),
                bytes: Vec::new(),
            },
            HolderMessage::Exited(ExitOutcome::Failed(ExitCause::Signal(
                SignalName::try_new("sigkill").unwrap(),
            ))),
            HolderMessage::Rejected(RejectReason::ReadOnly),
            HolderMessage::Warning("screen reset".to_string()),
            HolderMessage::Attached(3),
        ];
        for message in holder {
            let mut wire = Vec::new();
            message.clone().write_to(&mut wire).unwrap();
            assert_eq!(
                HolderMessage::read_from(&mut Cursor::new(wire)).unwrap(),
                message
            );
        }
    }

    /// A frame meant for the other direction is refused, not misread.
    #[test]
    fn a_frame_for_the_other_direction_is_refused() {
        let mut wire = Vec::new();
        HolderMessage::Output {
            at: OutputCursor::new(0),
            bytes: b"x".to_vec(),
        }
        .write_to(&mut wire)
        .unwrap();
        assert!(matches!(
            ClientMessage::read_from(&mut Cursor::new(wire)),
            Err(HolderError::UnexpectedFrame(FrameTag::Output))
        ));
    }

    /// Output too short to hold its offset is malformed.
    #[test]
    fn output_without_its_offset_is_malformed() {
        let frame = Frame::builder()
            .tag(FrameTag::Output)
            .payload(b"short".to_vec())
            .build();
        assert!(matches!(
            HolderMessage::try_from(frame),
            Err(HolderError::MalformedPayload(FrameTag::Output))
        ));
    }

    /// A snapshot naming pid 0 or init is malformed, so no client ever
    /// signals a pid that reaches beyond one child.
    #[test]
    fn a_snapshot_naming_no_child_pid_is_malformed() {
        for pid in [0_u32, 1] {
            let mut payload = pid.to_be_bytes().to_vec();
            payload.extend_from_slice(&[0, 24, 0, 80]);
            payload.extend_from_slice(&0_u32.to_be_bytes());
            payload.extend_from_slice(&0_u64.to_be_bytes());
            let frame = Frame::builder()
                .tag(FrameTag::Snapshot)
                .payload(payload)
                .build();
            assert!(matches!(
                HolderMessage::try_from(frame),
                Err(HolderError::MalformedPayload(FrameTag::Snapshot))
            ));
        }
    }

    /// A resize to an empty dimension is malformed, never applied.
    #[test]
    fn an_empty_resize_is_malformed() {
        let frame = Frame::builder()
            .tag(FrameTag::Resize)
            .payload(vec![0, 0, 0, 80])
            .build();
        assert!(matches!(
            ClientMessage::try_from(frame),
            Err(HolderError::MalformedPayload(FrameTag::Resize))
        ));
    }

    /// A resize past the PTY bounds, or narrower than a wide character, is
    /// malformed, so no client can make the holder allocate a huge screen.
    #[test]
    fn an_out_of_bounds_resize_is_malformed() {
        for payload in [vec![0xFF, 0xFF, 0xFF, 0xFF], vec![0, 24, 0, 1]] {
            let frame = Frame::builder()
                .tag(FrameTag::Resize)
                .payload(payload)
                .build();
            assert!(matches!(
                ClientMessage::try_from(frame),
                Err(HolderError::MalformedPayload(FrameTag::Resize))
            ));
        }
    }

    /// Structured bodies use kebab-case on the wire.
    #[test]
    fn roles_serialize_in_kebab_case() {
        let json = serde_json::to_string(&ClientRole::Attach { read_only: true }).unwrap();
        assert_eq!(json, r#"{"attach":{"read-only":true}}"#);
    }
}
