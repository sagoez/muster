use std::{
    collections::VecDeque,
    io,
    os::unix::net::UnixStream,
    sync::{
        Arc,
        mpsc::{Receiver, RecvTimeoutError},
    },
    time::{Duration, Instant},
};

use super::{
    attach_end::AttachEnd, attach_event::AttachEvent, detach_scanner::DetachScanner,
    error::CliError, held_terminal::AUTOWRAP_ON, host_terminal::HostTerminal,
    raw_mode_guard::RawModeGuard, session_alarm::SessionAlarm,
};
use crate::{
    adapter::{
        holder::{ClientMessage, HolderError, HolderMessage, RejectReason, client::Greeting},
        terminal::emulator::{DISABLE_AUTOWRAP, RESET_TERMINAL},
    },
    domain::{pty::PtySize, value::ProcessName},
};

/// How long an escape sequence cut off at the end of what was typed is held
/// for the rest before it is passed on as typed, so a lone Escape is never
/// held longer, however busy the process is.
const ESCAPE_WAIT: Duration = Duration::from_millis(20);
/// How long a session whose terminal differs in size from the process waits
/// for the screen redrawn at its size before drawing the one it has.
pub(super) const RESYNC_WAIT: Duration = Duration::from_secs(1);
/// How long a session whose input closed waits for the hangup signal that
/// usually follows, so the client ends by that signal.
const HANGUP_WAIT: Duration = Duration::from_millis(200);
/// How long after the first resize of a burst the terminal's size is passed
/// on, so a window being dragged resizes the process a few times rather than
/// at every step, and continuous output never holds a resize back.
const RESIZE_WAIT: Duration = Duration::from_millis(30);
/// Why a session ends when one of its helper threads panicked.
const HELPER_FAILED: &str = "a thread of the attach session failed";
/// How often a waiting session checks its alarm, for a report the inbox
/// had no room to carry.
const ALARM_CHECK: Duration = Duration::from_millis(100);

/// One attached session's live state: the holder connection, the terminal
/// in raw mode and the detach key's scanner. Dropping it gives the terminal
/// back.
pub(super) struct AttachPump<'a> {
    /// The process.
    name: &'a ProcessName,
    /// The terminal, for its size.
    terminal: &'a HostTerminal,
    /// The holder connection.
    stream: UnixStream,
    /// The terminal in raw mode.
    screen: RawModeGuard,
    /// Finds the detach key in what is typed.
    scanner: DetachScanner,
    /// Whether keystrokes and resizes are kept from the process.
    read_only: bool,
    /// When the escape sequence the scanner holds started being held.
    held_since: Option<Instant>,
    /// When the first resize not yet passed on arrived, so a burst of them
    /// is passed on once.
    resize_since: Option<Instant>,
    /// Events that arrived while waiting for the first screen, applied after
    /// it.
    backlog: VecDeque<AttachEvent>,
    /// Whether the holder dropped typed input because the process was not
    /// reading it.
    dropped_input: bool,
    /// What the helpers and signal listener report besides the inbox.
    alarm: Arc<SessionAlarm>,
}

impl<'a> AttachPump<'a> {
    /// A session for the process `name` over `stream`, drawing on `screen`.
    pub(super) fn new(
        name: &'a ProcessName,
        terminal: &'a HostTerminal,
        stream: UnixStream,
        screen: RawModeGuard,
        scanner: DetachScanner,
        read_only: bool,
        alarm: Arc<SessionAlarm>,
    ) -> Self {
        Self {
            name,
            terminal,
            stream,
            screen,
            scanner,
            read_only,
            alarm,
            held_since: None,
            resize_since: None,
            backlog: VecDeque::new(),
            dropped_input: false,
        }
    }

    /// Whether the holder dropped typed input during the session.
    pub(super) fn dropped_input(&self) -> bool {
        self.dropped_input
    }

    /// Draws the process's first screen. A writable session whose terminal
    /// differs in size first resizes the process and draws the screen the
    /// holder redraws at that size, falling back to `greeting`'s when none
    /// comes or the resize is refused; otherwise `greeting`'s is drawn and
    /// the size claimed after.
    ///
    /// # Errors
    /// Returns a [`CliError`] when the terminal or the holder cannot be
    /// written, or the connection breaks.
    pub(super) fn first_screen(
        &mut self,
        greeting: &Greeting,
        inbox: &Receiver<AttachEvent>,
    ) -> Result<(), CliError> {
        let own = self.terminal.size();
        let rows = own.unwrap_or_else(|| greeting.size()).rows().into_inner();
        self.screen
            .clear_for_first_screen(rows)
            .map_err(CliError::Terminal)?;
        match own {
            Some(size) if !self.read_only && size != greeting.size() => {
                self.send(ClientMessage::Resize(size))?;
                match self.await_resync(inbox)? {
                    Some((repaint, size)) => self.paint(&repaint, size),
                    None => self.paint(greeting.repaint(), greeting.size()),
                }
            },
            Some(size) if !self.read_only => {
                self.paint(greeting.repaint(), greeting.size())?;
                self.send(ClientMessage::Resize(size))
            },
            _ => self.paint(greeting.repaint(), greeting.size()),
        }
    }

    /// Waits at most [`RESYNC_WAIT`] for the screen the holder redraws after
    /// a resize. Output before it belongs to the old size and is dropped;
    /// anything else waits in the backlog. Answers none when the resize was
    /// refused or nothing came.
    ///
    /// # Errors
    /// Returns [`CliError::AttachLost`] when the connection breaks.
    fn await_resync(
        &mut self,
        inbox: &Receiver<AttachEvent>,
    ) -> Result<Option<(Vec<u8>, PtySize)>, CliError> {
        let deadline = Instant::now() + RESYNC_WAIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match inbox.recv_timeout(left) {
                Ok(AttachEvent::Holder(HolderMessage::Snapshot { repaint, size, .. })) => {
                    return Ok(Some((repaint, size)));
                },
                Ok(AttachEvent::Holder(HolderMessage::Rejected(_))) => return Ok(None),
                Ok(AttachEvent::Holder(HolderMessage::Output(_))) => {},
                Ok(AttachEvent::HolderLost(source)) => return Err(self.lost(source)),
                Ok(event) => self.backlog.push_back(event),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                    return Ok(None);
                },
            }
        }
    }

    /// Reacts to events until one ends the session.
    ///
    /// # Errors
    /// Returns a [`CliError`] when the connection breaks or the terminal
    /// cannot be written.
    pub(super) fn run(&mut self, inbox: &Receiver<AttachEvent>) -> Result<AttachEnd, CliError> {
        loop {
            if self.alarm.helper_failed() {
                return Err(self.helper_failed());
            }
            if let Some(signal) = self.alarm.ending() {
                return Ok(AttachEnd::Signalled(signal));
            }
            self.flush_escape_when_due()?;
            self.resize_when_due()?;
            let deadline = [
                self.held_since.map(|since| since + ESCAPE_WAIT),
                self.resize_since.map(|since| since + RESIZE_WAIT),
                Some(Instant::now() + ALARM_CHECK),
            ]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or_else(Instant::now);
            let event = if let Some(event) = self.backlog.pop_front() {
                event
            } else {
                match inbox.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(event) => event,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => return Ok(AttachEnd::Detached),
                }
            };
            if let Some(end) = self.handle(event, inbox)? {
                return Ok(end);
            }
        }
    }

    /// Passes the terminal's size on once [`RESIZE_WAIT`] has passed since
    /// the first resize of a burst, however busy the process is.
    ///
    /// # Errors
    /// Returns [`CliError::AttachLost`] when the holder cannot be written.
    fn resize_when_due(&mut self) -> Result<(), CliError> {
        match self.resize_since {
            Some(since) if since.elapsed() >= RESIZE_WAIT => {
                self.resize_since = None;
                self.resize()
            },
            _ => Ok(()),
        }
    }

    /// Passes a held escape on once it has been held for [`ESCAPE_WAIT`],
    /// whatever else arrived meanwhile.
    ///
    /// # Errors
    /// Returns [`CliError::AttachLost`] when the holder cannot be written.
    fn flush_escape_when_due(&mut self) -> Result<(), CliError> {
        match self.held_since {
            Some(since) if since.elapsed() >= ESCAPE_WAIT => {
                self.held_since = None;
                let held = self.scanner.flush();
                self.forward(held)
            },
            _ => Ok(()),
        }
    }

    /// Applies one event, answering how the session ends when it does.
    ///
    /// # Errors
    /// Returns a [`CliError`] when the connection breaks or the terminal
    /// cannot be written.
    fn handle(
        &mut self,
        event: AttachEvent,
        inbox: &Receiver<AttachEvent>,
    ) -> Result<Option<AttachEnd>, CliError> {
        match event {
            AttachEvent::Typed(bytes) => {
                let scanned = self.scanner.feed(&bytes);
                self.held_since = self.scanner.has_pending().then(Instant::now);
                self.forward(scanned.forward().clone())?;
                Ok(scanned.detached().then_some(AttachEnd::Detached))
            },
            AttachEvent::InputClosed => Ok(Some(await_hangup(inbox))),
            AttachEvent::Holder(HolderMessage::Output(bytes)) => {
                self.screen.write(&bytes).map_err(CliError::Terminal)?;
                Ok(None)
            },
            AttachEvent::Holder(HolderMessage::Snapshot { repaint, size, .. }) => {
                self.screen.clear_for_resync().map_err(CliError::Terminal)?;
                self.paint(&repaint, size)?;
                Ok(None)
            },
            AttachEvent::Holder(HolderMessage::Exited(outcome)) => {
                Ok(Some(AttachEnd::Exited(outcome)))
            },
            AttachEvent::Holder(HolderMessage::Rejected(reason)) => {
                self.dropped_input |= reason == RejectReason::InputBacklogged;
                Ok(None)
            },
            AttachEvent::Holder(HolderMessage::Warning(_) | HolderMessage::Attached(_)) => Ok(None),
            AttachEvent::HolderLost(source) => Err(self.lost(source)),
            AttachEvent::HelperFailed => Err(self.helper_failed()),
            AttachEvent::Resized => {
                self.resize_since.get_or_insert_with(Instant::now);
                Ok(None)
            },
            AttachEvent::Ended(signal) => Ok(Some(AttachEnd::Signalled(signal))),
        }
    }

    /// Draws a screen rendered at `size`. One that does not fit the terminal
    /// is drawn with line wrapping off, so a wider screen is cut at the
    /// terminal's edge rather than wrapped into its history.
    ///
    /// # Errors
    /// Returns [`CliError::Terminal`] when the terminal cannot be written.
    fn paint(&mut self, repaint: &[u8], size: PtySize) -> Result<(), CliError> {
        let repaint = repaint
            .strip_prefix(RESET_TERMINAL.as_bytes())
            .unwrap_or(repaint);
        if self.terminal.size().is_none_or(|own| own == size) {
            return self.screen.write(repaint).map_err(CliError::Terminal);
        }
        let keeps_wrap_off = repaint
            .windows(DISABLE_AUTOWRAP.len())
            .any(|window| window == DISABLE_AUTOWRAP.as_bytes());
        self.screen
            .write(DISABLE_AUTOWRAP.as_bytes())
            .and_then(|()| self.screen.write(repaint))
            .and_then(|()| {
                if keeps_wrap_off {
                    Ok(())
                } else {
                    self.screen.write(AUTOWRAP_ON.as_bytes())
                }
            })
            .map_err(CliError::Terminal)
    }

    /// Passes typed `bytes` on, unless the session is read-only.
    ///
    /// # Errors
    /// Returns [`CliError::AttachLost`] when the holder cannot be written.
    fn forward(&mut self, bytes: Vec<u8>) -> Result<(), CliError> {
        if self.read_only || bytes.is_empty() {
            return Ok(());
        }
        self.send(ClientMessage::Input(bytes))
    }

    /// Sizes the process to the terminal, unless the session is read-only.
    ///
    /// # Errors
    /// Returns [`CliError::AttachLost`] when the holder cannot be written.
    fn resize(&mut self) -> Result<(), CliError> {
        match self.terminal.size() {
            Some(size) if !self.read_only => self.send(ClientMessage::Resize(size)),
            _ => Ok(()),
        }
    }

    /// Sends `message` to the holder.
    ///
    /// # Errors
    /// Returns [`CliError::AttachLost`] when the holder cannot be written.
    fn send(&mut self, message: ClientMessage) -> Result<(), CliError> {
        message
            .write_to(&mut self.stream)
            .map_err(|source| self.lost(source))
    }

    /// The failure of a session whose helper thread failed: its connection
    /// can no longer be served.
    fn helper_failed(&self) -> CliError {
        self.lost(HolderError::Io(io::Error::other(HELPER_FAILED)))
    }

    /// The failure of a connection that broke with `source`.
    fn lost(&self, source: HolderError) -> CliError {
        CliError::AttachLost {
            name: self.name.clone(),
            source,
        }
    }
}

/// How a session whose input closed ends: by the signal that ended the
/// client when one comes within [`HANGUP_WAIT`], as a hangup's does; by the
/// process's exit when that comes instead; as a detach otherwise.
fn await_hangup(inbox: &Receiver<AttachEvent>) -> AttachEnd {
    let deadline = Instant::now() + HANGUP_WAIT;
    let mut exited = None;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match inbox.recv_timeout(left) {
            Ok(AttachEvent::Ended(signal)) => return AttachEnd::Signalled(signal),
            Ok(AttachEvent::Holder(HolderMessage::Exited(outcome))) => exited = Some(outcome),
            Ok(_) => {},
            Err(_) => return exited.map_or(AttachEnd::Detached, AttachEnd::Exited),
        }
    }
}
