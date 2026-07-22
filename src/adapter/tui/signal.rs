use std::{collections::HashMap, mem};

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD},
};

use crate::domain::notification::NotificationId;

/// Bell control byte (`\a`); the universal "I want your attention" signal, and
/// what most CLI agents (Claude Code among them) ring when waiting on the user.
const BELL: u8 = 0x07;
/// Escape control byte, used to introduce OSC and String Terminator sequences.
const ESCAPE: u8 = 0x1B;
/// Control byte that cancels an in-progress terminal sequence.
const CANCEL: u8 = 0x18;
/// Control byte that substitutes for an invalid terminal sequence.
const SUBSTITUTE: u8 = 0x1A;
/// The byte after Escape that introduces an OSC sequence.
const OSC_INTRODUCER: u8 = b']';
/// OSC 0 sets both the terminal icon name and title.
const OSC_ICON_AND_TITLE: &[u8] = b"0";
/// OSC 2 sets the terminal title.
const OSC_TITLE: &[u8] = b"2";
/// OSC 9: iTerm2-style notification, `OSC 9 ; <text>`.
const OSC_ITERM2: &[u8] = b"9";
/// OSC 99: kitty-style rich notification with metadata and a chunked payload.
const OSC_KITTY: &[u8] = b"99";
/// OSC 777: rxvt-unicode style, `OSC 777 ; notify ; <title> ; <body>`.
const OSC_RXVT: &[u8] = b"777";
/// Subcode marking `OSC 9 ; 4 ; <state> ; <progress>` (ConEmu progress) apart
/// from a plain OSC 9 notification.
const OSC_PROGRESS_SUBCODE: &[u8] = b"4";
/// Progress state `0` clears the bar, meaning the task finished.
const PROGRESS_DONE: &[u8] = b"0";
/// The `notify` keyword that opens an rxvt OSC 777 payload.
const RXVT_NOTIFY: &[u8] = b"notify";
/// Kitty payload-type value for a title chunk (also the default).
const KITTY_TITLE: &str = "title";
/// Kitty payload-type value for a body chunk.
const KITTY_BODY: &str = "body";
/// Kitty payload-type value for closing a previous notification.
const KITTY_CLOSE: &str = "close";
/// Cap on partial (incomplete) kitty notifications held at once.
const MAX_PENDING_KITTY: usize = 32;
/// Cap on encoded plus decoded bytes retained for one kitty title or body.
const MAX_KITTY_PAYLOAD: usize = 4096;
/// Cap on one raw OSC sequence. Kitty payloads are limited to 4096 encoded bytes;
/// the larger bound also accommodates long OSC 9 and OSC 777 messages safely.
const MAX_OSC_BYTES: usize = 64 * 1024;
/// Cap on terminal titles retained for activity detection.
const MAX_TITLE_BYTES: usize = 1024;
/// The String Terminator's final byte (`ESC \`).
const STRING_TERMINATOR: u8 = b'\\';

/// A signal decoded from a process's terminal output, in stream order so a
/// caller can apply them as they occurred rather than all at once.
pub enum Signal {
    /// Visible terminal output was produced (printing, cursor moves, and so on).
    /// Consecutive output is coalesced into a single signal.
    Output,
    /// The process changed its terminal title.
    Title(String),
    /// The process asked for attention: a bell, or an OSC 9/99/777 notification.
    Notify {
        /// Kitty identifier for replacing this notification, when present.
        identifier: Option<NotificationId>,
        /// Summary line, when the sequence carried one.
        title: Option<String>,
        /// Message body, when the sequence carried one.
        body: Option<String>,
    },
    /// The process asked to close a prior Kitty notification by identifier.
    Close {
        /// Identifier of the notification to close.
        identifier: NotificationId,
    },
    /// An OSC 9;4 progress report: `true` while a task runs, `false` when done.
    Progress(bool),
}

/// Current position in the lightweight OSC framing state machine.
#[derive(Clone, Copy, Default)]
enum StreamState {
    /// Ordinary terminal bytes.
    #[default]
    Ground,
    /// An Escape was seen; the next byte determines whether OSC begins.
    Escape,
    /// Bytes inside an OSC sequence.
    Osc,
    /// Escape seen inside OSC; dispatch waits for the `\\` completing ST.
    OscEscape,
    /// An oversized OSC is ignored until its terminator.
    DiscardOsc,
}

/// Partial text for one kitty title or body. Encoded fragments remain encoded
/// until a complete Base64 stream is available; decoded bytes are validated as
/// UTF-8 only when the whole notification completes.
#[derive(Default)]
struct KittyText {
    bytes: Vec<u8>,
    encoded: Vec<u8>,
}

impl KittyText {
    /// Appends one plain or Base64-encoded payload fragment within the byte cap.
    /// Returns `false` when the fragment is invalid or would exceed the cap.
    fn append(&mut self, payload: &[u8], encoded: bool) -> bool {
        if encoded {
            if !self.can_retain(payload.len()) {
                return false;
            }
            self.encoded.extend_from_slice(payload);
            // Padding marks the end of a Base64 stream. This supports clients
            // that chunk before encoding while unpadded fragments remain queued
            // for clients that chunk one stream after encoding.
            !payload.contains(&b'=') || self.flush_encoded()
        } else {
            self.flush_encoded() && self.can_retain(payload.len()) && {
                self.bytes.extend_from_slice(payload);
                true
            }
        }
    }

    /// Decodes and appends the queued Base64 stream, accepting omitted padding.
    /// Returns `false` when the stream is invalid or exceeds the retained cap.
    fn flush_encoded(&mut self) -> bool {
        if self.encoded.is_empty() {
            return true;
        }
        let encoded = mem::take(&mut self.encoded);
        let Some(decoded) = decode_kitty_base64(&encoded) else {
            return false;
        };
        if !self.can_retain(decoded.len()) {
            return false;
        }
        self.bytes.extend(decoded);
        true
    }

    /// Whether `additional` bytes fit within this field's retained byte cap.
    fn can_retain(&self, additional: usize) -> bool {
        self.len()
            .checked_add(additional)
            .is_some_and(|length| length <= MAX_KITTY_PAYLOAD)
    }

    /// Number of encoded and decoded bytes currently retained.
    fn len(&self) -> usize {
        self.bytes.len() + self.encoded.len()
    }
}

/// Partial title and body for one kitty notification id.
#[derive(Default)]
struct KittyNotification {
    title: KittyText,
    body: KittyText,
}

impl KittyNotification {
    /// Appends a supported text payload. Returns `false` for invalid or oversized
    /// input; unsupported payload types are filtered before this method is called.
    fn append(&mut self, payload_type: &str, payload: &[u8], encoded: bool) -> bool {
        match payload_type {
            KITTY_TITLE => self.title.append(payload, encoded),
            KITTY_BODY => self.body.append(payload, encoded),
            _ => true,
        }
    }

    /// Finishes Base64 decoding and UTF-8 validation for the complete title and
    /// body. Returns `None` when either text field is malformed.
    fn finish(mut self) -> Option<(String, String)> {
        if !self.title.flush_encoded() || !self.body.flush_encoded() {
            return None;
        }
        let title = String::from_utf8(self.title.bytes).ok()?;
        let body = String::from_utf8(self.body.bytes).ok()?;
        Some((title, body))
    }
}

/// Decodes title, notification, and progress signals from a single process's PTY
/// stream. Raw OSC framing is retained across calls so payload text is never
/// constrained by VTE's bounded parameter view. Screen rendering remains with
/// the separate vt100 parser.
pub struct SignalReader {
    state: StreamState,
    osc: Vec<u8>,
    kitty: HashMap<String, KittyNotification>,
}

impl SignalReader {
    /// Creates a reader with fresh framing and no partial notifications.
    pub fn new() -> Self {
        Self {
            state: StreamState::Ground,
            osc: Vec::new(),
            kitty: HashMap::new(),
        }
    }

    /// Feeds `bytes` and returns the signals they completed, in stream order.
    pub fn read(&mut self, bytes: &[u8]) -> Vec<Signal> {
        let mut signals = Vec::new();
        for &byte in bytes {
            self.advance(byte, &mut signals);
        }
        signals
    }

    /// Advances raw OSC framing by one byte and emits completed signals.
    fn advance(&mut self, byte: u8, signals: &mut Vec<Signal>) {
        match self.state {
            StreamState::Ground => self.advance_ground(byte, signals),
            StreamState::Escape => self.advance_escape(byte, signals),
            StreamState::Osc => self.advance_osc(byte, signals),
            StreamState::OscEscape => self.advance_osc_escape(byte, signals),
            StreamState::DiscardOsc => self.advance_discarded_osc(byte),
        }
    }

    /// Handles one byte outside an escape sequence.
    fn advance_ground(&mut self, byte: u8, signals: &mut Vec<Signal>) {
        match byte {
            ESCAPE => self.state = StreamState::Escape,
            BELL => push_bell(signals),
            _ => push_output(signals),
        }
    }

    /// Handles one byte after Escape, starting OSC or recording generic terminal
    /// activity. C0 controls execute without consuming the pending Escape.
    fn advance_escape(&mut self, byte: u8, signals: &mut Vec<Signal>) {
        match byte {
            OSC_INTRODUCER => {
                self.osc.clear();
                self.state = StreamState::Osc;
            },
            ESCAPE => {},
            STRING_TERMINATOR => self.state = StreamState::Ground,
            CANCEL | SUBSTITUTE => self.state = StreamState::Ground,
            BELL => push_bell(signals),
            0x00..=0x17 | 0x19 | 0x1C..=0x1F => push_output(signals),
            _ => {
                push_output(signals);
                self.state = StreamState::Ground;
            },
        }
    }

    /// Collects one raw OSC byte, dispatching on BEL, waiting for a complete ST,
    /// or discarding the sequence on cancellation.
    fn advance_osc(&mut self, byte: u8, signals: &mut Vec<Signal>) {
        match byte {
            BELL => {
                self.finish_osc(signals);
                self.state = StreamState::Ground;
            },
            ESCAPE => self.state = StreamState::OscEscape,
            CANCEL | SUBSTITUTE => {
                self.osc.clear();
                self.state = StreamState::Ground;
            },
            0x00..=0x06 | 0x08..=0x17 | 0x19 | 0x1C..=0x1F => {},
            _ if self.osc.len() < MAX_OSC_BYTES => self.osc.push(byte),
            _ => {
                self.osc.clear();
                self.state = StreamState::DiscardOsc;
            },
        }
    }

    /// Confirms the second byte of an OSC String Terminator. Any other escape
    /// continuation abandons the OSC and is handled as a fresh terminal escape.
    fn advance_osc_escape(&mut self, byte: u8, signals: &mut Vec<Signal>) {
        match byte {
            STRING_TERMINATOR => {
                self.finish_osc(signals);
                self.state = StreamState::Ground;
            },
            CANCEL | SUBSTITUTE => {
                self.osc.clear();
                self.state = StreamState::Ground;
            },
            _ => {
                self.osc.clear();
                self.state = StreamState::Escape;
                self.advance_escape(byte, signals);
            },
        }
    }

    /// Ignores an oversized OSC until BEL or Escape terminates it.
    fn advance_discarded_osc(&mut self, byte: u8) {
        match byte {
            BELL | CANCEL | SUBSTITUTE => self.state = StreamState::Ground,
            ESCAPE => self.state = StreamState::Escape,
            _ => {},
        }
    }

    /// Dispatches the complete raw OSC payload and releases its buffer.
    fn finish_osc(&mut self, signals: &mut Vec<Signal>) {
        let raw = mem::take(&mut self.osc);
        self.dispatch_osc(&raw, signals);
    }

    /// Parses one complete raw OSC payload without a bounded parameter split.
    fn dispatch_osc(&mut self, raw: &[u8], signals: &mut Vec<Signal>) {
        let Some((code, payload)) = split_once(raw, b';') else {
            return;
        };
        if matches!(code, OSC_ICON_AND_TITLE | OSC_TITLE) {
            self.title(payload, signals);
        } else if code == OSC_ITERM2 {
            self.iterm(payload, signals);
        } else if code == OSC_RXVT {
            self.rxvt(payload, signals);
        } else if code == OSC_KITTY {
            self.kitty(payload, signals);
        }
    }

    /// Emits a bounded terminal-title update for provider activity detection.
    fn title(&self, payload: &[u8], signals: &mut Vec<Signal>) {
        if payload.len() <= MAX_TITLE_BYTES {
            signals.push(Signal::Title(decode(payload)));
        }
    }

    /// Parses an iTerm2 notification or ConEmu progress OSC payload.
    fn iterm(&self, payload: &[u8], signals: &mut Vec<Signal>) {
        if let Some((subcode, progress)) = split_once(payload, b';')
            && subcode == OSC_PROGRESS_SUBCODE
        {
            let state = split_once(progress, b';')
                .map(|(state, _)| state)
                .unwrap_or(progress);
            signals.push(Signal::Progress(state != PROGRESS_DONE));
        } else {
            signals.push(Signal::Notify {
                identifier: None,
                title: None,
                body: Some(decode(payload)),
            });
        }
    }

    /// Parses an rxvt OSC 777 title and complete, unbounded body payload.
    fn rxvt(&self, payload: &[u8], signals: &mut Vec<Signal>) {
        let Some((command, content)) = split_once(payload, b';') else {
            return;
        };
        if command != RXVT_NOTIFY {
            return;
        }
        let (title, body) = match split_once(content, b';') {
            Some((title, body)) => (decode_non_empty(title), decode_non_empty(body)),
            None => (decode_non_empty(content), None),
        };
        signals.push(Signal::Notify {
            identifier: None,
            title,
            body,
        });
    }

    /// Accumulates a kitty title or body by notification id and emits it when
    /// `d` marks the notification complete. Unsupported payload types are ignored
    /// before text decoding but may still complete an accumulated notification.
    fn kitty(&mut self, payload: &[u8], signals: &mut Vec<Signal>) {
        let Some((metadata, raw_payload)) = split_once(payload, b';') else {
            return;
        };
        let meta = parse_meta(metadata);
        let identifier = match meta.get("i") {
            Some(value) => match NotificationId::try_new(value.clone()) {
                Ok(identifier) => Some(identifier),
                Err(_) => return,
            },
            None => None,
        };
        let id = identifier
            .as_ref()
            .map(|identifier| identifier.as_ref().to_string())
            .unwrap_or_default();
        let payload_type = meta.get("p").map(String::as_str).unwrap_or(KITTY_TITLE);
        if payload_type == KITTY_CLOSE {
            if let Some(identifier) = identifier {
                self.kitty.remove(&id);
                signals.push(Signal::Close { identifier });
            }
            return;
        }
        let text = matches!(payload_type, KITTY_TITLE | KITTY_BODY);
        let done = meta.get("d").map(|value| value != "0").unwrap_or(true);

        if text {
            self.make_room_for_kitty(&id);
            let encoded = meta.get("e").is_some_and(|value| value == "1");
            let valid = self.kitty.entry(id.clone()).or_default().append(
                payload_type,
                raw_payload,
                encoded,
            );
            if !valid {
                self.kitty.remove(&id);
                return;
            }
        }

        if done
            && let Some(notification) = self.kitty.remove(&id)
            && let Some((title, body)) = notification.finish()
            && (!title.is_empty() || !body.is_empty())
        {
            signals.push(Signal::Notify {
                identifier,
                title: (!title.is_empty()).then_some(title),
                body: (!body.is_empty()).then_some(body),
            });
        }
    }

    /// Makes room for a new partial Kitty identifier by evicting one abandoned
    /// entry at capacity. Existing identifiers retain their accumulated chunks.
    fn make_room_for_kitty(&mut self, id: &str) {
        if self.kitty.contains_key(id) || self.kitty.len() < MAX_PENDING_KITTY {
            return;
        }
        let abandoned = self.kitty.keys().next().cloned();
        if let Some(abandoned) = abandoned {
            self.kitty.remove(&abandoned);
        }
    }
}

impl Default for SignalReader {
    fn default() -> Self {
        Self::new()
    }
}

/// Records visible output, coalescing consecutive output into one signal.
fn push_output(signals: &mut Vec<Signal>) {
    if !matches!(signals.last(), Some(Signal::Output)) {
        signals.push(Signal::Output);
    }
}

/// Records a bare bell notification without title or body text.
fn push_bell(signals: &mut Vec<Signal>) {
    signals.push(Signal::Notify {
        identifier: None,
        title: None,
        body: None,
    });
}

/// Splits `bytes` at the first `separator`, retaining every later byte verbatim.
fn split_once(bytes: &[u8], separator: u8) -> Option<(&[u8], &[u8])> {
    let index = bytes.iter().position(|byte| *byte == separator)?;
    Some((&bytes[..index], &bytes[index + 1..]))
}

/// Decodes OSC text as UTF-8, replacing invalid sequences.
fn decode(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Decodes OSC text and treats an empty delimited field as absent.
fn decode_non_empty(bytes: &[u8]) -> Option<String> {
    let text = decode(bytes);
    (!text.is_empty()).then_some(text)
}

/// Decodes an RFC 4648 Base64 kitty byte stream with optional final padding.
fn decode_kitty_base64(payload: &[u8]) -> Option<Vec<u8>> {
    STANDARD
        .decode(payload)
        .or_else(|_| STANDARD_NO_PAD.decode(payload))
        .ok()
}

/// Parses a kitty metadata field containing colon-separated `key=value` pairs.
fn parse_meta(bytes: &[u8]) -> HashMap<String, String> {
    decode(bytes)
        .split(':')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Number of semicolon-delimited pieces needed to exceed VTE's OSC view.
    const MANY_OSC_FIELDS: usize = 32;

    /// Reads all chunks through one reader and collects signals in stream order.
    fn signals(chunks: &[&[u8]]) -> Vec<Signal> {
        let mut reader = SignalReader::new();
        chunks.iter().flat_map(|chunk| reader.read(chunk)).collect()
    }

    /// Collects notification and progress events while dropping plain output.
    fn events(chunks: &[&[u8]]) -> Vec<Signal> {
        signals(chunks)
            .into_iter()
            .filter(|signal| !matches!(signal, Signal::Output))
            .collect()
    }

    /// Builds text with enough semicolons to exceed a bounded OSC parameter list.
    fn many_fields() -> String {
        (0..MANY_OSC_FIELDS)
            .map(|index| format!("part{index}"))
            .collect::<Vec<_>>()
            .join(";")
    }

    #[test]
    fn osc_9_is_a_notification_body() {
        assert!(matches!(
            events(&[b"\x1b]9;build finished\x07"]).as_slice(),
            [Signal::Notify { title: None, body: Some(b), .. }] if b == "build finished"
        ));
    }

    /// Both standard title-setting OSC codes emit title signals.
    #[test]
    fn osc_0_and_2_report_terminal_titles() {
        assert!(matches!(
            events(&[b"\x1b]0;Codex working\x07", b"\x1b]2;Codex idle\x1b\\"]).as_slice(),
            [Signal::Title(first), Signal::Title(second)]
                if first == "Codex working" && second == "Codex idle"
        ));
    }

    #[test]
    fn osc_777_carries_title_and_body() {
        assert!(matches!(
            events(&[b"\x1b]777;notify;Claude;done thinking\x07"]).as_slice(),
            [Signal::Notify { title: Some(t), body: Some(b), .. }]
                if t == "Claude" && b == "done thinking"
        ));
    }

    #[test]
    fn osc_777_treats_an_empty_body_as_title_only() {
        assert!(matches!(
            events(&[b"\x1b]777;notify;Build done;\x07"]).as_slice(),
            [Signal::Notify { title: Some(title), body: None, .. }]
                if title == "Build done"
        ));
    }

    #[test]
    fn osc_777_treats_an_empty_title_as_body_only() {
        assert!(matches!(
            events(&[b"\x1b]777;notify;;Build done\x07"]).as_slice(),
            [Signal::Notify { title: None, body: Some(body), .. }]
                if body == "Build done"
        ));
    }

    #[test]
    fn osc_9_4_reports_progress_then_completion() {
        assert!(matches!(events(&[b"\x1b]9;4;1;40\x07"]).as_slice(), [
            Signal::Progress(true)
        ]));
        assert!(matches!(events(&[b"\x1b]9;4;0\x07"]).as_slice(), [
            Signal::Progress(false)
        ]));
    }

    #[test]
    fn a_bare_bell_is_a_notification_without_text() {
        assert!(matches!(events(&[b"\x07"]).as_slice(), [Signal::Notify {
            title: None,
            body: None,
            ..
        }]));
    }

    #[test]
    fn a_sequence_split_across_reads_is_still_recognised() {
        assert!(matches!(
            events(&[b"\x1b", b"]9;hel", b"lo world\x1b", b"\\"]).as_slice(),
            [Signal::Notify { body: Some(b), .. }] if b == "hello world"
        ));
    }

    #[test]
    fn an_osc_ending_after_escape_remains_incomplete() {
        let mut reader = SignalReader::new();
        assert!(reader.read(b"\x1b]9;not yet\x1b").is_empty());
        assert!(matches!(reader.read(b"\\").as_slice(), [
            Signal::Notify { body: Some(body), .. }
        ] if body == "not yet"));
    }

    #[test]
    fn a_non_terminating_escape_discards_the_partial_osc() {
        assert!(matches!(
            events(&[
                b"\x1b]9;must not display\x1b",
                b"[31m",
                b"\x1b]9;display this\x07",
            ])
            .as_slice(),
            [Signal::Notify { body: Some(body), .. }] if body == "display this"
        ));
    }

    #[test]
    fn can_and_sub_cancel_a_pending_escape() {
        for cancel in [CANCEL, SUBSTITUTE] {
            let mut input = vec![ESCAPE, cancel];
            input.extend_from_slice(b"]9;must not display\x1b\\");
            assert!(events(&[&input]).is_empty());
        }
    }

    #[test]
    fn canceled_osc_sequences_are_discarded() {
        for cancel in [CANCEL, SUBSTITUTE] {
            let mut input = b"\x1b]9;must not display".to_vec();
            input.push(cancel);
            assert!(events(&[&input]).is_empty());
        }
    }

    #[test]
    fn a_kitty_notification_chunks_title_and_body_by_id() {
        let out = events(&[
            b"\x1b]99;i=1:d=0;Claude\x1b\\",
            b"\x1b]99;i=1:p=body;finished\x1b\\",
        ]);
        assert!(matches!(
            out.as_slice(),
            [Signal::Notify { title: Some(t), body: Some(b), .. }]
                if t == "Claude" && b == "finished"
        ));
    }

    #[test]
    fn kitty_updates_and_close_preserve_the_identifier() {
        let out = events(&[
            b"\x1b]99;i=build;first\x1b\\",
            b"\x1b]99;i=build;second\x1b\\",
            b"\x1b]99;i=build:p=close;\x1b\\",
        ]);
        assert!(matches!(
            out.as_slice(),
            [
                Signal::Notify { identifier: Some(first), .. },
                Signal::Notify { identifier: Some(second), .. },
                Signal::Close { identifier: closed },
            ] if first.as_ref() == "build"
                && second.as_ref() == "build"
                && closed.as_ref() == "build"
        ));
    }

    #[test]
    fn a_single_sequence_kitty_notification_is_a_title() {
        assert!(matches!(
            events(&[b"\x1b]99;;Hello world\x07"]).as_slice(),
            [Signal::Notify { title: Some(t), body: None, .. }] if t == "Hello world"
        ));
    }

    #[test]
    fn a_kitty_payload_keeps_its_semicolons() {
        assert!(matches!(
            events(&[b"\x1b]99;p=body;a;b;c\x07"]).as_slice(),
            [Signal::Notify { body: Some(b), .. }] if b == "a;b;c"
        ));
    }

    #[test]
    fn a_kitty_notification_decodes_base64_title_and_body() {
        let out = events(&[
            b"\x1b]99;i=1:d=0:e=1;Q2xhdWRl\x1b\\",
            b"\x1b]99;i=1:p=body:e=1;ZmluaXNoZWQ=\x1b\\",
        ]);
        assert!(matches!(
            out.as_slice(),
            [Signal::Notify { title: Some(t), body: Some(b), .. }]
                if t == "Claude" && b == "finished"
        ));
    }

    #[test]
    fn an_unpadded_base64_kitty_payload_is_accepted() {
        assert!(matches!(
            events(&[b"\x1b]99;e=1;SGVsbG8\x1b\\"]).as_slice(),
            [Signal::Notify { title: Some(t), body: None, .. }] if t == "Hello"
        ));
    }

    #[test]
    fn a_base64_stream_split_between_quanta_is_reassembled() {
        let out = events(&[
            b"\x1b]99;i=1:d=0:e=1;SGV\x1b\\",
            b"\x1b]99;i=1:d=0:e=1;sbG8gd\x1b\\",
            b"\x1b]99;i=1:e=1;29ybGQ=\x1b\\",
        ]);
        assert!(matches!(
            out.as_slice(),
            [Signal::Notify { title: Some(t), body: None, .. }] if t == "Hello world"
        ));
    }

    #[test]
    fn a_binary_icon_does_not_discard_accumulated_text() {
        let out = events(&[
            b"\x1b]99;i=1:d=0;Claude\x1b\\",
            b"\x1b]99;i=1:p=icon:e=1;iVBORw0KGgo=\x1b\\",
        ]);
        assert!(matches!(
            out.as_slice(),
            [Signal::Notify { title: Some(t), body: None, .. }] if t == "Claude"
        ));
    }

    #[test]
    fn osc_text_beyond_the_parameter_limit_is_preserved() {
        let text = many_fields();
        let iterm = format!("\x1b]9;{text}\x07");
        assert!(matches!(
            events(&[iterm.as_bytes()]).as_slice(),
            [Signal::Notify { body: Some(body), .. }] if body == &text
        ));

        let kitty = format!("\x1b]99;p=body;{text}\x07");
        assert!(matches!(
            events(&[kitty.as_bytes()]).as_slice(),
            [Signal::Notify { body: Some(body), .. }] if body == &text
        ));

        let rxvt = format!("\x1b]777;notify;title;{text}\x07");
        assert!(matches!(
            events(&[rxvt.as_bytes()]).as_slice(),
            [Signal::Notify { body: Some(body), .. }] if body == &text
        ));
    }

    #[test]
    fn abandoned_kitty_notifications_do_not_grow_unbounded() {
        let mut reader = SignalReader::new();
        for i in 0..1000 {
            reader.read(format!("\x1b]99;i={i}:d=0;partial\x1b\\").as_bytes());
        }
        assert!(reader.kitty.len() <= MAX_PENDING_KITTY);
    }

    #[test]
    fn kitty_notifications_continue_after_pending_map_saturation() {
        let mut reader = SignalReader::new();
        for identifier in 0..MAX_PENDING_KITTY {
            reader.read(format!("\x1b]99;i={identifier}:d=0;abandoned\x1b\\").as_bytes());
        }

        assert!(matches!(
            reader.read(b"\x1b]99;i=fresh;still delivered\x1b\\").as_slice(),
            [Signal::Notify { title: Some(title), .. }] if title == "still delivered"
        ));
        assert!(reader.kitty.len() <= MAX_PENDING_KITTY);
    }

    #[test]
    fn a_deferred_payload_under_one_id_stays_bounded() {
        let mut reader = SignalReader::new();
        for _ in 0..1000 {
            reader.read(b"\x1b]99;i=1:d=0;xxxxxxxxxxxxxxxx\x1b\\");
        }
        let accumulated = reader
            .kitty
            .values()
            .map(|notification| notification.title.len() + notification.body.len())
            .sum::<usize>();
        assert!(
            accumulated <= MAX_KITTY_PAYLOAD,
            "one never-completed id cannot grow without limit: {accumulated}"
        );
    }

    #[test]
    fn an_st_terminated_notification_has_no_trailing_output() {
        assert!(matches!(signals(&[b"\x1b]9;hi\x1b\\"]).as_slice(), [
            Signal::Notify { .. }
        ]));
    }

    #[test]
    fn output_after_a_bell_is_ordered_after_it() {
        assert!(matches!(signals(&[b"\x07done\n"]).as_slice(), [
            Signal::Notify { .. },
            Signal::Output
        ]));
    }
}
