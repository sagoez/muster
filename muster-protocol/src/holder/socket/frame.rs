use std::io::{self, Read, Write};

use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::frame_tag::FrameTag;

/// Largest payload a frame may carry, in bytes. Bounds what one peer can make
/// the other allocate; a full-screen snapshot stays far below it.
pub const MAX_FRAME_BYTES: u32 = 16 * 1024 * 1024;
/// Bytes in a frame header: the tag followed by the big-endian payload length.
const HEADER_BYTES: usize = 5;
/// Reported for a payload above [`MAX_FRAME_BYTES`].
const OVERSIZED_PAYLOAD: &str = "holder frame payload exceeds the maximum size";
/// Reported for a tag byte no [`FrameTag`] carries.
const UNKNOWN_TAG: &str = "holder frame has an unknown tag";

/// One tagged, length-prefixed message on a holder socket.
#[derive(Debug, Clone, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct Frame {
    /// What the payload holds.
    #[getset(get_copy = "pub")]
    tag: FrameTag,
    /// The message body.
    #[getset(get = "pub")]
    payload: Vec<u8>,
}

impl Frame {
    /// Bytes the frame occupies on the wire, header included.
    pub fn wire_len(&self) -> usize {
        HEADER_BYTES + self.payload.len()
    }

    /// Consumes the frame, returning its payload.
    pub fn into_payload(self) -> Vec<u8> {
        self.payload
    }
}

/// Writes `frame` as a tag byte, a 4-byte big-endian length and the payload.
///
/// # Errors
/// Returns [`io::ErrorKind::InvalidInput`] when the payload exceeds
/// [`MAX_FRAME_BYTES`], or the underlying write error.
pub fn write_frame(writer: &mut impl Write, frame: &Frame) -> io::Result<()> {
    let length = u32::try_from(frame.payload.len())
        .ok()
        .filter(|length| *length <= MAX_FRAME_BYTES)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, OVERSIZED_PAYLOAD))?;
    let mut header = [0u8; HEADER_BYTES];
    header[0] = frame.tag as u8;
    header[1..].copy_from_slice(&length.to_be_bytes());
    writer.write_all(&header)?;
    writer.write_all(&frame.payload)?;
    writer.flush()
}

/// Reads one frame written by [`write_frame`].
///
/// # Errors
/// Returns [`io::ErrorKind::InvalidData`] for an unknown tag or a payload above
/// [`MAX_FRAME_BYTES`], [`io::ErrorKind::UnexpectedEof`] when the peer closes,
/// or the underlying read error.
pub fn read_frame(reader: &mut impl Read) -> io::Result<Frame> {
    let mut header = [0u8; HEADER_BYTES];
    reader.read_exact(&mut header)?;
    let tag = FrameTag::from_repr(header[0])
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, UNKNOWN_TAG))?;
    let mut length = [0u8; HEADER_BYTES - 1];
    length.copy_from_slice(&header[1..]);
    let length = u32::from_be_bytes(length);
    if length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            OVERSIZED_PAYLOAD,
        ));
    }
    let mut payload = Vec::new();
    reader.take(u64::from(length)).read_to_end(&mut payload)?;
    if payload.len() != length as usize {
        return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
    }
    Ok(Frame { tag, payload })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn frame_round_trips_and_rejects_oversized_payload() {
        let frame = Frame::builder()
            .tag(FrameTag::Output)
            .payload(b"hello\x1b[0m".to_vec())
            .build();
        let mut wire = Vec::new();
        write_frame(&mut wire, &frame).unwrap();
        assert_eq!(read_frame(&mut Cursor::new(&wire)).unwrap(), frame);

        let mut oversized = vec![FrameTag::Output as u8];
        oversized.extend_from_slice(&(MAX_FRAME_BYTES + 1).to_be_bytes());
        let error = read_frame(&mut Cursor::new(oversized)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);

        let too_big = Frame::builder()
            .tag(FrameTag::Output)
            .payload(vec![0; MAX_FRAME_BYTES as usize + 1])
            .build();
        let error = write_frame(&mut Vec::new(), &too_big).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn an_unknown_tag_or_truncated_payload_is_refused() {
        let unknown = [0xFF, 0, 0, 0, 0];
        let error = read_frame(&mut Cursor::new(unknown)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);

        let truncated = [FrameTag::Input as u8, 0, 0, 0, 4, b'a'];
        let error = read_frame(&mut Cursor::new(truncated)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }
}
