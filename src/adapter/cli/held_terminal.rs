use std::{
    fs::File,
    io::{self, Write},
};

use rustix::termios::{OptionalActions, Termios, tcgetattr, tcsetattr};

use super::host_terminal::HostTerminal;
use crate::adapter::{
    sanitize::printable,
    terminal::{
        escape::SGR_RESET,
        mode_scanner::{ModeScanner, Observation},
        screen_snapshot::cursor_position,
    },
};

/// Saves the window title on the terminal's title stack.
pub(super) const PUSH_TITLE: &str = "\x1b[22;0t";
/// Restores the window title saved by [`PUSH_TITLE`].
pub(super) const POP_TITLE: &str = "\x1b[23;0t";
/// Opens a window title (OSC 2).
const TITLE_START: &str = "\x1b]2;";
/// Ends a window title (BEL).
const TITLE_END: &str = "\x07";
/// Leaves the alternate screen, restoring the cursor saved on entry.
pub(super) const LEAVE_ALT_SCREEN: &str = "\x1b[?1049l";
/// Pops every Kitty keyboard level a process pushed, then clears the flags
/// of the level beneath; a terminal without the protocol ignores both.
pub(super) const KITTY_KEYBOARD_RESET: &str = "\x1b[<64u\x1b[=0;1u";
/// Designates ASCII as G0 and shifts into G0, undoing a line-drawing
/// charset.
const CHARSET_ASCII: &str = "\x1b(B\x0f";
/// Turns insert mode off.
const INSERT_MODE_OFF: &str = "\x1b[4l";
/// Turns automatic newline mode (LNM) off.
const NEWLINE_MODE_OFF: &str = "\x1b[20l";
/// Turns reverse video off.
const REVERSE_VIDEO_OFF: &str = "\x1b[?5l";
/// Turns bracketed paste off.
const BRACKETED_PASTE_OFF: &str = "\x1b[?2004l";
/// Turns every mouse reporting mode and encoding off.
const MOUSE_OFF: &str = "\x1b[?9l\x1b[?1000l\x1b[?1001l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l\x1b[?1015l\x1b[?1016l";
/// Turns focus reporting off.
const FOCUS_REPORTING_OFF: &str = "\x1b[?1004l";
/// Returns the cursor keys and keypad to their normal encodings.
const KEYS_NORMAL: &str = "\x1b[?1l\x1b>";
/// Returns xterm's modifyOtherKeys to its default.
const MODIFY_OTHER_KEYS_DEFAULT: &str = "\x1b[>4m";
/// Clears the scroll region, saving and restoring the cursor around it since
/// clearing it homes the cursor.
const SCROLL_REGION_RESET: &str = "\x1b7\x1b[r\x1b8";
/// Turns line wrapping back on.
pub(super) const AUTOWRAP_ON: &str = "\x1b[?7h";
/// Returns the cursor to the terminal's default shape.
const CURSOR_SHAPE_DEFAULT: &str = "\x1b[0 q";
/// Shows the cursor.
const CURSOR_SHOWN: &str = "\x1b[?25h";
/// Turns origin mode off, which also homes the cursor; sent only when the
/// cursor is homed anyway or origin mode is known to be on.
pub(super) const ORIGIN_MODE_OFF: &str = "\x1b[?6l";
/// What returns a terminal's primary screen to how a shell expects it,
/// whatever the process turned on, without moving the cursor.
pub(super) const MODES_RESET: [&str; 15] = [
    CHARSET_ASCII,
    INSERT_MODE_OFF,
    NEWLINE_MODE_OFF,
    REVERSE_VIDEO_OFF,
    BRACKETED_PASTE_OFF,
    MOUSE_OFF,
    FOCUS_REPORTING_OFF,
    KEYS_NORMAL,
    MODIFY_OTHER_KEYS_DEFAULT,
    SCROLL_REGION_RESET,
    SGR_RESET,
    AUTOWRAP_ON,
    CURSOR_SHAPE_DEFAULT,
    CURSOR_SHOWN,
    KITTY_KEYBOARD_RESET,
];
/// Moves the cursor home.
const CURSOR_HOME: &str = "\x1b[H";
/// Erases the whole screen.
const ERASE_SCREEN: &str = "\x1b[2J";
/// Starts a fresh line below the process's screen and clears what is below
/// it, so the shell continues on a clean line.
pub(super) const SESSION_END: &str = "\r\n\x1b[J";
/// A line feed, which scrolls the screen once the cursor is on its last row.
const LINE_FEED: &str = "\n";
/// The first column, one-based.
const FIRST_COLUMN: u16 = 1;

/// A terminal in raw mode and what was drawn on it, shared between its
/// [`RawModeGuard`](super::RawModeGuard) and the panic hook that may have to
/// give it back first.
pub struct HeldTerminal {
    /// Where the screen is drawn, and whose settings raw mode changes.
    output: File,
    /// The terminal settings before raw mode.
    original: Termios,
    /// Follows what is drawn for screen and origin-mode switches.
    scanner: ModeScanner,
    /// Whether what was drawn left the terminal on its alternate screen.
    alternate: bool,
    /// Whether the terminal was already given back.
    restored: bool,
}

impl HeldTerminal {
    /// Puts `terminal` in raw mode and saves its window title.
    ///
    /// # Errors
    /// Returns the I/O error when the terminal cannot be read, switched or
    /// written.
    pub fn enter(terminal: &HostTerminal) -> io::Result<Self> {
        let output = File::from(terminal.output().try_clone()?);
        let original = tcgetattr(&output)?;
        let mut raw = original.clone();
        raw.make_raw();
        let mut held = Self {
            output,
            original,
            scanner: ModeScanner::new(),
            alternate: false,
            restored: true,
        };
        tcsetattr(&held.output, OptionalActions::Now, &raw)?;
        held.restored = false;
        held.write(PUSH_TITLE.as_bytes())?;
        Ok(held)
    }

    /// Draws `bytes`, following them for screen and origin-mode switches.
    ///
    /// # Errors
    /// Returns the I/O error when the terminal cannot be written.
    pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        for byte in bytes {
            match self.scanner.feed(*byte) {
                Observation::EnterAlternate => self.alternate = true,
                Observation::LeaveAlternate | Observation::FullReset => self.alternate = false,
                Observation::Nothing
                | Observation::ScrollRegion { .. }
                | Observation::SetTabStop
                | Observation::ClearTabStop
                | Observation::ClearAllTabStops
                | Observation::ShiftOut
                | Observation::ShiftIn
                | Observation::Dropped => {},
            }
        }
        self.output.write_all(bytes)
    }

    /// Shows `title` as the window title, made [`printable`] so it cannot end
    /// the title early or steer the terminal.
    ///
    /// # Errors
    /// Returns the I/O error when the terminal cannot be written.
    pub fn set_title(&mut self, title: &str) -> io::Result<()> {
        let title = printable(title);
        self.write(format!("{TITLE_START}{title}{TITLE_END}").as_bytes())
    }

    /// Readies the terminal for a process's first screen: modes back to
    /// defaults, then the `rows` on display scrolled into the terminal's
    /// history rather than erased, so what was there before stays reachable.
    ///
    /// # Errors
    /// Returns the I/O error when the terminal cannot be written.
    pub fn clear_for_first_screen(&mut self, rows: u16) -> io::Result<()> {
        self.reset(true)?;
        let mut sequence = cursor_position(rows, FIRST_COLUMN);
        sequence.push_str(&LINE_FEED.repeat(usize::from(rows)));
        sequence.push_str(CURSOR_HOME);
        self.write(sequence.as_bytes())
    }

    /// Readies the terminal for a process's screen sent again: modes back to
    /// defaults and the screen erased.
    ///
    /// # Errors
    /// Returns the I/O error when the terminal cannot be written.
    pub fn clear_for_resync(&mut self) -> io::Result<()> {
        self.reset(true)?;
        self.write([CURSOR_HOME, ERASE_SCREEN].concat().as_bytes())
    }

    /// Brings the terminal back to its primary screen with default modes and
    /// no Kitty keyboard levels: the alternate screen's levels are popped
    /// before leaving it, then the primary screen's. Leaving first lets the
    /// cursor it restores bring back its origin mode, which is then turned
    /// off when it is on, or when `homing`, since the cursor is homed next
    /// anyway.
    ///
    /// # Errors
    /// Returns the I/O error when the terminal cannot be written.
    fn reset(&mut self, homing: bool) -> io::Result<()> {
        if self.alternate {
            self.write([KITTY_KEYBOARD_RESET, LEAVE_ALT_SCREEN].concat().as_bytes())?;
        }
        let mut sequence = String::new();
        if homing || self.scanner.origin_mode() {
            sequence.push_str(ORIGIN_MODE_OFF);
        }
        sequence.extend(MODES_RESET);
        self.write(sequence.as_bytes())
    }

    /// Gives the terminal back as it was found, once.
    ///
    /// # Errors
    /// Returns the first I/O error; the settings are restored even when the
    /// reset cannot be written.
    pub fn restore(&mut self) -> io::Result<()> {
        if self.restored {
            return Ok(());
        }
        self.restored = true;
        let written = self
            .reset(false)
            .and_then(|()| self.write([SESSION_END, POP_TITLE].concat().as_bytes()));
        let settings = tcsetattr(&self.output, OptionalActions::Now, &self.original);
        written.and(settings.map_err(io::Error::from))
    }
}
