//! Terminal emulation for a managed pane, backed by `alacritty_terminal`.
//!
//! This wraps an `alacritty_terminal::Term` and its VT parser behind the small
//! screen-model surface the holder and service need: feed bytes, read the screen
//! text, and repaint the screen with its modes onto a fresh terminal.

use std::{
    mem,
    panic::{self, AssertUnwindSafe},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use alacritty_terminal::{
    Term,
    event::{Event, EventListener, VoidListener},
    grid::{Cursor, Dimensions, Grid},
    index::{Column, Line},
    term::{
        Config, TermMode,
        cell::{Cell, Flags},
        test::TermSize,
    },
    vte::ansi::{
        self, CharsetIndex, Color as VtColor, CursorShape, NamedColor, Rgb, StandardCharset,
    },
};
use getset::CopyGetters;
use typed_builder::TypedBuilder;

use super::{
    cursor_state::CursorState,
    escape::{CSI, SCROLL_REGION_FINAL},
    mode_scanner::{ModeScanner, Observation},
    screen_snapshot::{ScreenSnapshot, SnapshotCell, cursor_position, push_charsets},
    tab_stops::TabStops,
};
use crate::domain::{
    color_scheme::ColorScheme,
    value::{Cols, Rows},
};

/// Full terminal reset (RIS), the first byte sequence of a repaint.
pub const RESET_TERMINAL: &str = "\x1bc";
/// DECSET that switches to the alternate screen and saves the cursor.
const ENTER_ALT_SCREEN: &str = "\x1b[?1049h";
/// DECSET that enables bracketed paste.
const ENABLE_BRACKETED_PASTE: &str = "\x1b[?2004h";
/// DECSET that enables application cursor keys.
const ENABLE_APP_CURSOR: &str = "\x1b[?1h";
/// DECSET that enables press and release mouse reporting.
const ENABLE_MOUSE_PRESS_RELEASE: &str = "\x1b[?1000h";
/// DECSET that enables button-motion mouse reporting.
const ENABLE_MOUSE_BUTTON_MOTION: &str = "\x1b[?1002h";
/// DECSET that enables any-motion mouse reporting.
const ENABLE_MOUSE_ANY_MOTION: &str = "\x1b[?1003h";
/// DECSET that selects the UTF-8 mouse encoding.
const ENABLE_MOUSE_UTF8: &str = "\x1b[?1005h";
/// DECSET that selects the SGR mouse encoding.
const ENABLE_MOUSE_SGR: &str = "\x1b[?1006h";
/// DECRST that hides the cursor.
const HIDE_CURSOR: &str = "\x1b[?25l";
/// DECRST that disables autowrap.
pub const DISABLE_AUTOWRAP: &str = "\x1b[?7l";
/// DECSET that enables focus in/out reporting.
const ENABLE_FOCUS_REPORTING: &str = "\x1b[?1004h";
/// DECSET that confines cursor addressing to the scroll region; it homes the
/// cursor.
const ENABLE_ORIGIN_MODE: &str = "\x1b[?6h";
/// SM that turns insert mode on.
const ENABLE_INSERT_MODE: &str = "\x1b[4h";
/// SO: makes G1 the active charset.
const SHIFT_OUT: &str = "\x0e";
/// TBC that clears every tab stop.
const CLEAR_ALL_TAB_STOPS: &str = "\x1b[3g";
/// HTS: sets a tab stop at the cursor.
const SET_TAB_STOP: &str = "\x1bH";
/// Final bytes of a DECSCUSR sequence (`CSI n SP q`) after its parameter.
const CURSOR_STYLE_FINAL: &str = " q";
/// DECSCUSR parameter for a blinking block cursor; steady forms follow by one.
const CURSOR_BLINKING_BLOCK: u8 = 1;
/// DECSCUSR parameter for a blinking underline cursor.
const CURSOR_BLINKING_UNDERLINE: u8 = 3;
/// DECSCUSR parameter for a blinking beam cursor.
const CURSOR_BLINKING_BEAM: u8 = 5;

/// xterm mouse-reporting mode a child has requested, mapped from the terminal's
/// private modes so pointer encoding does not depend on the emulator backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseProtocolMode {
    /// No mouse reporting.
    None,
    /// Report presses and releases (mode 1000).
    PressRelease,
    /// Report presses, releases, and motion while a button is held (mode 1002).
    ButtonMotion,
    /// Report presses, releases, and all motion (mode 1003).
    AnyMotion,
}

/// Byte encoding a child expects for mouse reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseProtocolEncoding {
    /// Legacy single-byte encoding.
    Default,
    /// UTF-8 extended coordinates (mode 1005).
    Utf8,
    /// SGR encoding (mode 1006).
    Sgr,
}

/// A cell color independent of any rendering frontend. Palette indices pass
/// through so the host terminal applies its own theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellColor {
    /// The terminal's default foreground or background.
    Default,
    /// An entry of the 256-color palette.
    Indexed(u8),
    /// A true-color value.
    Rgb(u8, u8, u8),
}

/// Text attributes of one cell.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, CopyGetters, TypedBuilder)]
pub struct CellFlags {
    /// Bold weight.
    #[getset(get_copy = "pub")]
    #[builder(default)]
    bold: bool,
    /// Dim intensity.
    #[getset(get_copy = "pub")]
    #[builder(default)]
    dim: bool,
    /// Italic slant.
    #[getset(get_copy = "pub")]
    #[builder(default)]
    italic: bool,
    /// Any underline style.
    #[getset(get_copy = "pub")]
    #[builder(default)]
    underline: bool,
    /// Inverse video.
    #[getset(get_copy = "pub")]
    #[builder(default)]
    inverse: bool,
    /// Concealed text.
    #[getset(get_copy = "pub")]
    #[builder(default)]
    hidden: bool,
    /// Struck-through text.
    #[getset(get_copy = "pub")]
    #[builder(default)]
    strikeout: bool,
}

/// The resolved colors and attributes of one cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, CopyGetters, TypedBuilder)]
pub struct CellStyle {
    /// Foreground color.
    #[getset(get_copy = "pub")]
    fg: CellColor,
    /// Background color.
    #[getset(get_copy = "pub")]
    bg: CellColor,
    /// Text attributes.
    #[getset(get_copy = "pub")]
    flags: CellFlags,
}

impl Default for CellStyle {
    /// The unstyled cell: default colors and no attributes.
    fn default() -> Self {
        Self::builder()
            .fg(CellColor::Default)
            .bg(CellColor::Default)
            .flags(CellFlags::default())
            .build()
    }
}

/// One grid cell as a test compares it: its glyph, combining characters,
/// resolved style, and whether it is the trailing half of a wide character.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq)]
pub struct RenderCell {
    ch: char,
    zerowidth: Vec<char>,
    style: CellStyle,
    spacer: bool,
}

#[cfg(test)]
impl RenderCell {
    /// The glyph in the cell.
    pub fn ch(&self) -> char {
        self.ch
    }

    /// Combining characters attached to the glyph.
    pub fn zerowidth(&self) -> &[char] {
        &self.zerowidth
    }
}

/// Runtime state the VT event listener records for the owning emulator to drain:
/// bytes the terminal wants written back to the child (device-status and cursor
/// reports, primary/secondary DA, and so on).
#[derive(Default)]
struct EmulatorSink {
    pty_writes: Vec<u8>,
}

/// Locks the sink, recovering the guard if a panic poisoned it: the buffer holds
/// plain bytes, so a half-finished push is harmless.
fn lock_sink(sink: &Mutex<EmulatorSink>) -> MutexGuard<'_, EmulatorSink> {
    sink.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The `alacritty_terminal` event listener. It shares [`EmulatorSink`] with the
/// emulator through `Arc<Mutex<_>>` so the emulator can move between threads.
#[derive(Clone)]
struct EventProxy {
    sink: Arc<Mutex<EmulatorSink>>,
    color_scheme: ColorScheme,
}

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        match event {
            // Status and identity reports the terminal wants written back verbatim.
            Event::PtyWrite(text) => lock_sink(&self.sink)
                .pty_writes
                .extend_from_slice(text.as_bytes()),
            // A child querying a color (OSC 10/11 for the default foreground and
            // background, OSC 4 for a palette entry) must get an answer, or an agent
            // like Codex, unable to read the background, themes for a light terminal
            // and renders white on white. Reply from a standard palette with the
            // configured scheme's defaults, the way a real terminal does.
            Event::ColorRequest(index, format) => {
                let reply = format(resolve_color(index, self.color_scheme));
                lock_sink(&self.sink)
                    .pty_writes
                    .extend_from_slice(reply.as_bytes());
            },
            _ => {},
        }
    }
}

/// A managed pane's terminal screen.
pub struct TerminalEmulator {
    term: Term<EventProxy>,
    parser: ansi::Processor,
    sink: Arc<Mutex<EmulatorSink>>,
    scanner: ModeScanner,
    /// A copy of the primary screen while the alternate one is shown, resized
    /// alongside it because the backend keeps the real one private.
    primary: Option<Term<VoidListener>>,
    scroll_region: Option<(usize, usize)>,
    /// Scrollback lines the screen keeps, which the primary copy keeps too.
    history: usize,
    /// The tab stops, which the backend keeps private.
    tabs: TabStops,
    /// Whether SO made G1 the active charset, which the backend keeps private.
    shift_out: bool,
    /// The colors color queries are answered from, kept to rebuild the screen.
    color_scheme: ColorScheme,
}

impl TerminalEmulator {
    /// Builds an emulator for a `rows` by `cols` screen retaining `scrollback`
    /// lines of history, answering color queries from `color_scheme`.
    pub fn new(rows: Rows, cols: Cols, scrollback: usize, color_scheme: ColorScheme) -> Self {
        let size = TermSize::new(cols.into_inner() as usize, rows.into_inner() as usize);
        Self::sized(size, scrollback, color_scheme)
    }

    /// Builds an emulator for a one-column screen, which no PTY takes, so a
    /// test can drive the backend into a panic with a wide character.
    #[cfg(test)]
    pub(crate) fn one_column(rows: Rows, scrollback: usize, color_scheme: ColorScheme) -> Self {
        Self::sized(
            TermSize::new(1, rows.into_inner() as usize),
            scrollback,
            color_scheme,
        )
    }

    /// Builds an emulator for a `size` screen retaining `scrollback` lines of
    /// history, answering color queries from `color_scheme`.
    fn sized(size: TermSize, scrollback: usize, color_scheme: ColorScheme) -> Self {
        let sink = Arc::new(Mutex::new(EmulatorSink::default()));
        let proxy = EventProxy {
            sink: sink.clone(),
            color_scheme,
        };
        let term = Term::new(history_config(scrollback), &size, proxy);
        Self {
            term,
            parser: ansi::Processor::new(),
            sink,
            scanner: ModeScanner::new(),
            primary: None,
            scroll_region: None,
            history: scrollback,
            tabs: TabStops::new(size.columns()),
            shift_out: false,
            color_scheme,
        }
    }

    /// Feeds `bytes` as [`Self::process`] does, but a panic in the backend
    /// leaves a blank screen of the same size in place of one it may have
    /// left corrupt, rather than taking the caller's thread down. Replies to
    /// queries the chunk answered before the panic survive the reset, for
    /// [`Self::take_pty_writes`]. Returns whether the screen was reset.
    pub fn process_or_reset(&mut self, bytes: &[u8]) -> bool {
        if panic::catch_unwind(AssertUnwindSafe(|| self.process(bytes))).is_ok() {
            return false;
        }
        let pending = self.take_pty_writes();
        let size = TermSize::new(self.term.columns(), self.term.screen_lines());
        *self = Self::sized(size, self.history, self.color_scheme);
        lock_sink(&self.sink).pty_writes = pending;
        true
    }

    /// Feeds one chunk of child output through the VT parser. The primary screen
    /// is copied just before an alternate-screen switch takes effect, so a
    /// later [`repaint`](Self::repaint) can rebuild both screens. A tab stop
    /// change is applied first, so the cursor column it acts on is known.
    pub fn process(&mut self, bytes: &[u8]) {
        let mut start = 0;
        for (index, byte) in bytes.iter().enumerate() {
            match self.scanner.feed(*byte) {
                Observation::Nothing | Observation::LeaveAlternate => {},
                Observation::EnterAlternate => {
                    self.parser.advance(&mut self.term, &bytes[start..index]);
                    start = index;
                    if !self.alternate_screen() {
                        self.primary = Some(self.copy_primary());
                    }
                },
                Observation::ScrollRegion { top, bottom } => {
                    let bottom = bottom.unwrap_or_else(|| self.term.screen_lines());
                    if top < bottom {
                        self.scroll_region = Some((top, bottom));
                    }
                },
                Observation::FullReset => {
                    self.scroll_region = None;
                    self.tabs = TabStops::new(self.term.columns());
                    self.shift_out = false;
                },
                Observation::ShiftOut => self.shift_out = true,
                Observation::ShiftIn => self.shift_out = false,
                Observation::Dropped => {
                    self.parser.advance(&mut self.term, &bytes[start..index]);
                    self.parser = ansi::Processor::new();
                    start = index + 1;
                },
                observation @ (Observation::SetTabStop
                | Observation::ClearTabStop
                | Observation::ClearAllTabStops) => {
                    self.parser.advance(&mut self.term, &bytes[start..=index]);
                    start = index + 1;
                    let column = self.term.grid().cursor.point.column.0;
                    match observation {
                        Observation::SetTabStop => self.tabs.set(column),
                        Observation::ClearTabStop => self.tabs.clear(column),
                        _ => self.tabs.clear_all(),
                    }
                },
            }
        }
        self.parser.advance(&mut self.term, &bytes[start..]);
        if !self.alternate_screen() {
            self.primary = None;
        }
    }

    /// A copy of the primary screen as it is now, rebuilt from its repaint.
    fn copy_primary(&self) -> Term<VoidListener> {
        let size = TermSize::new(self.term.columns(), self.term.screen_lines());
        let mut copy = Term::new(history_config(self.history), &size, VoidListener);
        let mut parser: ansi::Processor = ansi::Processor::new();
        parser.advance(&mut copy, &self.repaint());
        copy
    }

    /// Resizes the screen to `rows` by `cols`, reflowing history, and the
    /// primary screen behind the alternate one with it.
    pub fn set_size(&mut self, rows: Rows, cols: Cols) {
        let (rows, cols) = (rows.into_inner() as usize, cols.into_inner() as usize);
        self.tabs.resize(cols);
        if let Some(primary) = &mut self.primary {
            primary.resize(TermSize::new(cols, rows));
        }
        self.term.resize(TermSize::new(cols, rows));
        self.scroll_region = None;
    }

    /// The screen dimensions as `(rows, cols)`.
    pub fn size(&self) -> (u16, u16) {
        (self.term.screen_lines() as u16, self.term.columns() as u16)
    }

    /// Lines currently scrolled up into history (0 at the live bottom).
    pub fn scrollback(&self) -> usize {
        self.term.grid().display_offset()
    }

    /// Whether the child is on the alternate screen.
    pub fn alternate_screen(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    /// Whether the child requested bracketed paste (DECSET 2004), so a paste must
    /// be wrapped so it is treated as one paste rather than typed input.
    pub fn bracketed_paste(&self) -> bool {
        self.term.mode().contains(TermMode::BRACKETED_PASTE)
    }

    /// Whether application cursor keys (DECCKM) are active, which selects the
    /// arrow-key form sent for alternate-screen wheel scrolling.
    pub fn application_cursor(&self) -> bool {
        self.term.mode().contains(TermMode::APP_CURSOR)
    }

    /// The child's requested mouse-reporting mode.
    pub fn mouse_protocol_mode(&self) -> MouseProtocolMode {
        let mode = self.term.mode();
        if mode.contains(TermMode::MOUSE_MOTION) {
            MouseProtocolMode::AnyMotion
        } else if mode.contains(TermMode::MOUSE_DRAG) {
            MouseProtocolMode::ButtonMotion
        } else if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
            MouseProtocolMode::PressRelease
        } else {
            MouseProtocolMode::None
        }
    }

    /// The child's requested mouse-report byte encoding.
    pub fn mouse_protocol_encoding(&self) -> MouseProtocolEncoding {
        let mode = self.term.mode();
        if mode.contains(TermMode::SGR_MOUSE) {
            MouseProtocolEncoding::Sgr
        } else if mode.contains(TermMode::UTF8_MOUSE) {
            MouseProtocolEncoding::Utf8
        } else {
            MouseProtocolEncoding::Default
        }
    }

    /// Takes any bytes the terminal wants written back to the child (status and
    /// identity reports), leaving the buffer empty.
    pub fn take_pty_writes(&mut self) -> Vec<u8> {
        mem::take(&mut lock_sink(&self.sink).pty_writes)
    }

    /// The escape sequence that rebuilds this terminal on a fresh one: reset, the
    /// primary screen then the alternate screen when active, the tab stops,
    /// scroll region, cells, saved cursor, cursor, the modes the emulator
    /// reports, the charsets and finally the pen, so the same later output
    /// draws the same screen. Scrollback is not included, and the Kitty
    /// keyboard flags live in `KittyKeyboardTracker` because the emulator does
    /// not enable that protocol.
    pub fn repaint(&self) -> Vec<u8> {
        let (rows, cols) = self.size();
        let mut out = String::from(RESET_TERMINAL);
        if self.alternate_screen() {
            if let Some(primary) = &self.primary {
                let screen = capture(primary.grid());
                screen.paint_cells(rows, cols, &mut out);
                screen.place_cursor(rows, cols, 0, &mut out);
                screen.restore_pen(&mut out);
            }
            out.push_str(ENTER_ALT_SCREEN);
            push_charsets([StandardCharset::Ascii; 2], &mut out);
        }
        self.push_tab_stops(&mut out);
        if let Some((top, bottom)) = self.scroll_region {
            out.push_str(&format!("{CSI}{top};{bottom}{SCROLL_REGION_FINAL}"));
        }
        let screen = capture(self.term.grid());
        screen.paint_cells(rows, cols, &mut out);
        screen.save_cursor(rows, cols, &mut out);
        let mut origin_top = 0;
        if self.term.mode().contains(TermMode::ORIGIN) {
            out.push_str(ENABLE_ORIGIN_MODE);
            origin_top = self
                .scroll_region
                .map_or(0, |(top, _)| u16::try_from(top - 1).unwrap_or(0));
        }
        screen.place_cursor(rows, cols, origin_top, &mut out);
        self.push_mode_sequences(&mut out);
        if self.shift_out {
            out.push_str(SHIFT_OUT);
        }
        screen.restore_pen(&mut out);
        out.into_bytes()
    }

    /// Appends the sequences that replace the default tab stops with the
    /// child's, when it changed them.
    fn push_tab_stops(&self, out: &mut String) {
        if self.tabs.is_default() {
            return;
        }
        out.push_str(CLEAR_ALL_TAB_STOPS);
        for column in self.tabs.columns() {
            out.push_str(&cursor_position(
                1,
                u16::try_from(column + 1).unwrap_or(u16::MAX),
            ));
            out.push_str(SET_TAB_STOP);
        }
    }

    /// Appends the DECSCUSR sequence for the cursor style, when it has one.
    fn push_cursor_style(&self, out: &mut String) {
        let style = self.term.cursor_style();
        let blinking_code = match style.shape {
            CursorShape::Block => CURSOR_BLINKING_BLOCK,
            CursorShape::Underline => CURSOR_BLINKING_UNDERLINE,
            CursorShape::Beam => CURSOR_BLINKING_BEAM,
            CursorShape::HollowBlock | CursorShape::Hidden => return,
        };
        let code = if style.blinking {
            blinking_code
        } else {
            blinking_code + 1
        };
        out.push_str(&format!("{CSI}{code}{CURSOR_STYLE_FINAL}"));
    }

    /// Appends the private-mode sequences a repaint restores after the cursor.
    fn push_mode_sequences(&self, out: &mut String) {
        self.push_cursor_style(out);
        let mode = self.term.mode();
        if !mode.contains(TermMode::LINE_WRAP) {
            out.push_str(DISABLE_AUTOWRAP);
        }
        if mode.contains(TermMode::FOCUS_IN_OUT) {
            out.push_str(ENABLE_FOCUS_REPORTING);
        }
        if !mode.contains(TermMode::SHOW_CURSOR) {
            out.push_str(HIDE_CURSOR);
        }
        if self.bracketed_paste() {
            out.push_str(ENABLE_BRACKETED_PASTE);
        }
        if self.application_cursor() {
            out.push_str(ENABLE_APP_CURSOR);
        }
        if mode.contains(TermMode::INSERT) {
            out.push_str(ENABLE_INSERT_MODE);
        }
        match self.mouse_protocol_mode() {
            MouseProtocolMode::None => {},
            MouseProtocolMode::PressRelease => out.push_str(ENABLE_MOUSE_PRESS_RELEASE),
            MouseProtocolMode::ButtonMotion => out.push_str(ENABLE_MOUSE_BUTTON_MOTION),
            MouseProtocolMode::AnyMotion => out.push_str(ENABLE_MOUSE_ANY_MOTION),
        }
        match self.mouse_protocol_encoding() {
            MouseProtocolEncoding::Default => {},
            MouseProtocolEncoding::Utf8 => out.push_str(ENABLE_MOUSE_UTF8),
            MouseProtocolEncoding::Sgr => out.push_str(ENABLE_MOUSE_SGR),
        }
    }

    /// The visible cursor position as viewport `(row, col)`, or `None` when the
    /// cursor is hidden or the viewport is scrolled up into history.
    pub fn cursor(&self) -> Option<(u16, u16)> {
        if !self.term.mode().contains(TermMode::SHOW_CURSOR)
            || self.term.grid().display_offset() != 0
        {
            return None;
        }
        let point = self.term.grid().cursor.point;
        let row = point.line.0;
        (row >= 0).then_some((row as u16, point.column.0 as u16))
    }

    /// The cell at viewport `(row, col)`.
    #[cfg(test)]
    pub fn cell(&self, row: u16, col: u16) -> RenderCell {
        let line = Line(row as i32 - self.term.grid().display_offset() as i32);
        let cell = &self.term.grid()[line][Column(col as usize)];
        let flags = cell.flags;
        let spacer = flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER);
        RenderCell {
            ch: cell.c,
            zerowidth: cell.zerowidth().map(<[char]>::to_vec).unwrap_or_default(),
            style: cell_style(cell.fg, cell.bg, flags),
            spacer,
        }
    }

    /// The text of the linear selection from viewport `(top, start_col)` to
    /// `(bot, end_col)`: the first row from `start_col`, whole rows between, and
    /// the last row up to `end_col`, with per-line trailing blanks trimmed.
    pub fn contents_between(&self, top: u16, start_col: u16, bot: u16, end_col: u16) -> String {
        let cols = self.term.columns();
        let display = self.term.grid().display_offset() as i32;
        let grid = self.term.grid();
        let mut out = String::new();
        for row in top..=bot {
            let line = Line(row as i32 - display);
            let (from, to) = if top == bot {
                (start_col as usize, end_col as usize)
            } else if row == top {
                (start_col as usize, cols)
            } else if row == bot {
                (0, end_col as usize)
            } else {
                (0, cols)
            };
            let mut row_text = String::new();
            let mut col = from;
            let to = to.min(cols);
            while col < to {
                let cell = &grid[line][Column(col)];
                if !cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    row_text.push(cell.c);
                }
                col += 1;
            }
            out.push_str(row_text.trim_end_matches(' '));
            if row != bot {
                out.push('\n');
            }
        }
        out
    }

    /// The text on the pane's live bottom screen - the latest `rows` lines the
    /// process wrote - independent of any scrollback the human has scrolled to,
    /// so an agent observing a process always reads its most recent output.
    pub fn screen_text(&self) -> String {
        let (rows, cols) = self.size();
        if rows == 0 || cols == 0 {
            return String::new();
        }
        // `contents_between` reads viewport rows relative to the current
        // display offset; offsetting by that offset targets the active screen
        // (grid lines 0..rows), i.e. the live bottom, whatever the scroll.
        let display = self.scrollback() as u16;
        self.contents_between(display, 0, display + rows - 1, cols - 1)
    }
}

/// The config of a terminal keeping `scrollback` lines of history.
fn history_config(scrollback: usize) -> Config {
    Config {
        scrolling_history: scrollback,
        ..Config::default()
    }
}

/// Copies a screen's cells, cursor and saved cursor.
fn capture(grid: &Grid<Cell>) -> ScreenSnapshot {
    let rows = grid.screen_lines();
    let cols = grid.columns();
    let cells =
        (0..rows)
            .map(|row| {
                (0..cols)
                    .map(|col| {
                        let cell = &grid[Line(i32::try_from(row).unwrap_or(i32::MAX))][Column(col)];
                        SnapshotCell::builder()
                            .ch(cell.c)
                            .zerowidth(cell.zerowidth().map(<[char]>::to_vec).unwrap_or_default())
                            .style(cell_style(cell.fg, cell.bg, cell.flags))
                            .spacer(cell.flags.intersects(
                                Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER,
                            ))
                            .build()
                    })
                    .collect()
            })
            .collect();
    ScreenSnapshot::builder()
        .rows(cells)
        .cursor(cursor_state(&grid.cursor))
        .saved(cursor_state(&grid.saved_cursor))
        .build()
}

/// What a repaint needs of one `alacritty_terminal` cursor.
fn cursor_state(cursor: &Cursor<Cell>) -> CursorState {
    let point = cursor.point;
    let template = &cursor.template;
    CursorState::builder()
        .position((
            u16::try_from(point.line.0).unwrap_or(0),
            u16::try_from(point.column.0).unwrap_or(0),
        ))
        .wrap_pending(cursor.input_needs_wrap)
        .pen(cell_style(template.fg, template.bg, template.flags))
        .charsets([
            cursor.charsets[CharsetIndex::G0],
            cursor.charsets[CharsetIndex::G1],
        ])
        .build()
}

/// Resolves an `alacritty_terminal` cell's colors and attribute flags to a
/// [`CellStyle`]. Named and indexed colors pass through as palette indices so the
/// host terminal applies its own theme; only true-color specs become RGB.
fn cell_style(fg: VtColor, bg: VtColor, flags: Flags) -> CellStyle {
    CellStyle::builder()
        .fg(to_color(fg))
        .bg(to_color(bg))
        .flags(
            CellFlags::builder()
                .bold(flags.contains(Flags::BOLD))
                .dim(flags.contains(Flags::DIM))
                .italic(flags.contains(Flags::ITALIC))
                .underline(flags.intersects(Flags::ALL_UNDERLINES))
                .inverse(flags.contains(Flags::INVERSE))
                .hidden(flags.contains(Flags::HIDDEN))
                .strikeout(flags.contains(Flags::STRIKEOUT))
                .build(),
        )
        .build()
}

/// The largest `NamedColor` discriminant that maps to a 16-color palette index.
/// The named default foreground and background sit far above this (256 and 257),
/// so they - and cursor, dim, and bright aliases - resolve to the host default;
/// comparing as a wide integer avoids a `u8` cast wrapping 257 back into the
/// palette (which painted the default background red).
const LAST_ANSI_NAMED: usize = 15;

/// Maps an `alacritty_terminal` color to a [`CellColor`], keeping palette indices
/// so the host terminal's theme is honored.
fn to_color(color: VtColor) -> CellColor {
    match color {
        VtColor::Spec(rgb) => CellColor::Rgb(rgb.r, rgb.g, rgb.b),
        VtColor::Indexed(index) => CellColor::Indexed(index),
        VtColor::Named(named) => {
            let index = named as usize;
            if index <= LAST_ANSI_NAMED {
                CellColor::Indexed(index as u8)
            } else {
                CellColor::Default
            }
        },
    }
}

/// The standard xterm palette for the sixteen ANSI colors, reported for color
/// queries of indices 0-15.
const ANSI_PALETTE: [Rgb; 16] = [
    Rgb {
        r: 0x00,
        g: 0x00,
        b: 0x00,
    },
    Rgb {
        r: 0x80,
        g: 0x00,
        b: 0x00,
    },
    Rgb {
        r: 0x00,
        g: 0x80,
        b: 0x00,
    },
    Rgb {
        r: 0x80,
        g: 0x80,
        b: 0x00,
    },
    Rgb {
        r: 0x00,
        g: 0x00,
        b: 0x80,
    },
    Rgb {
        r: 0x80,
        g: 0x00,
        b: 0x80,
    },
    Rgb {
        r: 0x00,
        g: 0x80,
        b: 0x80,
    },
    Rgb {
        r: 0xC0,
        g: 0xC0,
        b: 0xC0,
    },
    Rgb {
        r: 0x80,
        g: 0x80,
        b: 0x80,
    },
    Rgb {
        r: 0xFF,
        g: 0x00,
        b: 0x00,
    },
    Rgb {
        r: 0x00,
        g: 0xFF,
        b: 0x00,
    },
    Rgb {
        r: 0xFF,
        g: 0xFF,
        b: 0x00,
    },
    Rgb {
        r: 0x00,
        g: 0x00,
        b: 0xFF,
    },
    Rgb {
        r: 0xFF,
        g: 0x00,
        b: 0xFF,
    },
    Rgb {
        r: 0x00,
        g: 0xFF,
        b: 0xFF,
    },
    Rgb {
        r: 0xFF,
        g: 0xFF,
        b: 0xFF,
    },
];

/// Background reported for an OSC 11 query under the dark scheme.
const DARK_BACKGROUND: Rgb = Rgb {
    r: 0x12,
    g: 0x12,
    b: 0x12,
};
/// Foreground reported for an OSC 10 query under the dark scheme.
const DARK_FOREGROUND: Rgb = Rgb {
    r: 0xD0,
    g: 0xD0,
    b: 0xD0,
};
/// Background reported for an OSC 11 query under the light scheme.
const LIGHT_BACKGROUND: Rgb = Rgb {
    r: 0xFA,
    g: 0xFA,
    b: 0xFA,
};
/// Foreground reported for an OSC 10 query under the light scheme.
const LIGHT_FOREGROUND: Rgb = Rgb {
    r: 0x10,
    g: 0x10,
    b: 0x10,
};

/// The default background and foreground `scheme` reports.
fn scheme_colors(scheme: ColorScheme) -> (Rgb, Rgb) {
    match scheme {
        ColorScheme::Dark => (DARK_BACKGROUND, DARK_FOREGROUND),
        ColorScheme::Light => (LIGHT_BACKGROUND, LIGHT_FOREGROUND),
    }
}

/// Resolves a color-query index to an RGB value: the xterm 256-color palette for
/// indices 0-255, and `scheme`'s background or foreground for the named
/// defaults.
fn resolve_color(index: usize, scheme: ColorScheme) -> Rgb {
    let (background, foreground) = scheme_colors(scheme);
    match index {
        0..=15 => ANSI_PALETTE[index],
        16..=231 => cube_color(index - 16),
        232..=255 => {
            let level = ((index - 232) * 10 + 8) as u8;
            Rgb {
                r: level,
                g: level,
                b: level,
            }
        },
        i if i == NamedColor::Background as usize => background,
        _ => foreground,
    }
}

/// One color of xterm's 6x6x6 color cube (palette indices 16-231, passed here as
/// a 0-215 offset).
fn cube_color(offset: usize) -> Rgb {
    let component = |value: usize| -> u8 {
        if value == 0 {
            0
        } else {
            (value * 40 + 55) as u8
        }
    };
    Rgb {
        r: component(offset / 36),
        g: component((offset / 6) % 6),
        b: component(offset % 6),
    }
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::{
        term::TermMode,
        vte::ansi::{Color as VtColor, NamedColor, Rgb},
    };

    use super::{
        CellColor, ColorScheme, DARK_BACKGROUND, LIGHT_BACKGROUND, TerminalEmulator, resolve_color,
        to_color,
    };
    use crate::{
        adapter::terminal::bounded_parser::MAX_PENDING_BYTES,
        domain::value::{Cols, Rows},
    };

    fn rows(count: u16) -> Rows {
        Rows::try_new(count).unwrap()
    }

    fn cols(count: u16) -> Cols {
        Cols::try_new(count).unwrap()
    }

    /// Repainting a screen onto a fresh emulator reproduces its cells, cursor, and
    /// tracked private modes, so a late viewer sees what the live pane shows.
    #[test]
    fn repaint_reproduces_cells_cursor_and_modes() {
        let mut live = TerminalEmulator::new(rows(5), cols(20), 100, ColorScheme::Dark);
        live.process(
            b"\x1b[31mred\x1b[0m plain\r\n\x1b[1;4mbold\x1b[?1049h\x1b[2;3Halt\x1b[?2004h",
        );
        let mut rebuilt = TerminalEmulator::new(rows(5), cols(20), 100, ColorScheme::Dark);
        rebuilt.process(&live.repaint());
        assert_eq!(rebuilt.screen_text(), live.screen_text());
        assert_eq!(rebuilt.cursor(), live.cursor());
        assert_eq!(rebuilt.alternate_screen(), live.alternate_screen());
        assert_eq!(rebuilt.bracketed_paste(), live.bracketed_paste());
        for row in 0..5 {
            for col in 0..20 {
                assert_eq!(rebuilt.cell(row, col), live.cell(row, col));
            }
        }
    }

    /// Styled primary-screen cells, true color, a hidden cursor, application
    /// cursor keys, and the mouse modes all survive a repaint.
    #[test]
    fn repaint_reproduces_styles_and_input_modes_on_the_primary_screen() {
        let mut live = TerminalEmulator::new(rows(4), cols(16), 0, ColorScheme::Dark);
        live.process(
            b"\x1b[38;2;10;20;30;48;5;200;3;9mhi\x1b[0m\r\nnext\x1b[?1h\x1b[?1002h\x1b[?1006h\x1b[?25l",
        );
        let mut rebuilt = TerminalEmulator::new(rows(4), cols(16), 0, ColorScheme::Dark);
        rebuilt.process(&live.repaint());
        assert_eq!(rebuilt.cursor(), None);
        assert!(rebuilt.application_cursor());
        assert_eq!(rebuilt.mouse_protocol_mode(), live.mouse_protocol_mode());
        assert_eq!(
            rebuilt.mouse_protocol_encoding(),
            live.mouse_protocol_encoding()
        );
        for row in 0..4 {
            for col in 0..16 {
                assert_eq!(rebuilt.cell(row, col), live.cell(row, col));
            }
        }
    }

    fn assert_screens_match(live: &TerminalEmulator, rebuilt: &TerminalEmulator) {
        assert_eq!(rebuilt.screen_text(), live.screen_text());
        assert_eq!(rebuilt.cursor(), live.cursor());
        let (rows, cols) = live.size();
        for row in 0..rows {
            for col in 0..cols {
                assert_eq!(
                    rebuilt.cell(row, col),
                    live.cell(row, col),
                    "cell {row},{col}"
                );
            }
        }
    }

    /// A client attaching during a full-screen app gets the primary screen back
    /// once the app leaves the alternate screen.
    #[test]
    fn repaint_restores_the_primary_screen_behind_the_alternate_screen() {
        let mut live = TerminalEmulator::new(rows(5), cols(20), 0, ColorScheme::Dark);
        live.process(b"\x1b[32mprompt$\x1b[0m ls\r\nfile.txt\x1b[?1049h\x1b[2;3Halt");
        live.process(b" screen");
        let mut rebuilt = TerminalEmulator::new(rows(5), cols(20), 0, ColorScheme::Dark);
        rebuilt.process(&live.repaint());
        assert!(rebuilt.alternate_screen());
        assert_screens_match(&live, &rebuilt);
        live.process(b"\x1b[?1049l");
        rebuilt.process(b"\x1b[?1049l");
        assert!(!rebuilt.alternate_screen());
        assert!(rebuilt.screen_text().contains("prompt$ ls"));
        assert_screens_match(&live, &rebuilt);
    }

    /// An alternate-screen switch split across output chunks still captures the
    /// primary screen.
    #[test]
    fn a_split_alternate_screen_switch_still_captures_the_primary_screen() {
        let mut live = TerminalEmulator::new(rows(4), cols(12), 0, ColorScheme::Dark);
        live.process(b"shell text\x1b[?10");
        live.process(b"49hfull");
        let mut rebuilt = TerminalEmulator::new(rows(4), cols(12), 0, ColorScheme::Dark);
        rebuilt.process(&live.repaint());
        live.process(b"\x1b[?1049l");
        rebuilt.process(b"\x1b[?1049l");
        assert_screens_match(&live, &rebuilt);
    }

    /// Combining accents and wide characters survive a repaint cell for cell.
    #[test]
    fn repaint_reproduces_combining_and_wide_characters() {
        let mut live = TerminalEmulator::new(rows(3), cols(12), 0, ColorScheme::Dark);
        live.process("e\u{301}x \u{754c}\u{9762}!\r\n\u{754c}".as_bytes());
        assert_eq!(live.cell(0, 0).zerowidth(), ['\u{301}']);
        let mut rebuilt = TerminalEmulator::new(rows(3), cols(12), 0, ColorScheme::Dark);
        rebuilt.process(&live.repaint());
        assert_eq!(rebuilt.cell(0, 0).zerowidth(), ['\u{301}']);
        assert_screens_match(&live, &rebuilt);
    }

    /// The scroll region, autowrap, focus reporting, and cursor style are
    /// restored, so the rebuilt terminal reacts to later output like the live one.
    #[test]
    fn repaint_reproduces_region_wrap_focus_and_cursor_style() {
        let mut live = TerminalEmulator::new(rows(5), cols(10), 0, ColorScheme::Dark);
        live.process(b"top\x1b[2;5r\x1b[?7l\x1b[?1004h\x1b[5 q\x1b[2;1Hone\r\ntwo");
        let mut rebuilt = TerminalEmulator::new(rows(5), cols(10), 0, ColorScheme::Dark);
        rebuilt.process(&live.repaint());
        assert!(!rebuilt.term.mode().contains(TermMode::LINE_WRAP));
        assert!(rebuilt.term.mode().contains(TermMode::FOCUS_IN_OUT));
        assert_eq!(rebuilt.term.cursor_style(), live.term.cursor_style());
        let later = b"\x1b[5;1H\r\nnext\r\nmore 0123456789abcdef";
        live.process(later);
        rebuilt.process(later);
        assert!(live.screen_text().starts_with("top"));
        assert_screens_match(&live, &rebuilt);
    }

    /// A repaint taken just after a line was filled to its last column keeps
    /// the wrap pending, so the next character starts the next line in the
    /// rebuilt terminal as it does in the live one. A resync snapshot taken
    /// at such a moment used to shift every later character by one.
    #[test]
    fn repaint_keeps_a_pending_wrap() {
        let mut live = TerminalEmulator::new(rows(3), cols(4), 0, ColorScheme::Dark);
        live.process(b"go\r\nabcd");
        let mut rebuilt = TerminalEmulator::new(rows(3), cols(4), 0, ColorScheme::Dark);
        rebuilt.process(&live.repaint());

        live.process(b"efg");
        rebuilt.process(b"efg");

        assert_screens_match(&live, &rebuilt);
        assert_eq!(live.cell(2, 0).ch(), 'e');
    }

    /// A pending wrap after a styled character is restored with that
    /// character's style, and the pen does not leak into later output.
    #[test]
    fn repaint_keeps_a_pending_wrap_after_a_styled_character() {
        let mut live = TerminalEmulator::new(rows(2), cols(3), 0, ColorScheme::Dark);
        live.process(b"ab\x1b[1mc\x1b[0m");
        let mut rebuilt = TerminalEmulator::new(rows(2), cols(3), 0, ColorScheme::Dark);
        rebuilt.process(&live.repaint());

        live.process(b"d");
        rebuilt.process(b"d");

        assert_screens_match(&live, &rebuilt);
    }

    /// A pending wrap after a wide character that fills the last two columns
    /// is restored by printing that character again, not its spacer.
    #[test]
    fn repaint_keeps_a_pending_wrap_after_a_wide_character() {
        let mut live = TerminalEmulator::new(rows(2), cols(4), 0, ColorScheme::Dark);
        live.process("ab\u{754c}".as_bytes());
        let mut rebuilt = TerminalEmulator::new(rows(2), cols(4), 0, ColorScheme::Dark);
        rebuilt.process(&live.repaint());

        live.process(b"c");
        rebuilt.process(b"c");

        assert_screens_match(&live, &rebuilt);
        assert_eq!(live.cell(1, 0).ch(), 'c');
    }

    /// Repaints a `rows` by `cols` emulator fed `setup` onto a fresh one, then
    /// feeds both `follow_up` and asserts they show the same screen.
    fn assert_follow_up_matches(rows_count: u16, cols_count: u16, setup: &[u8], follow_up: &[u8]) {
        let mut live =
            TerminalEmulator::new(rows(rows_count), cols(cols_count), 0, ColorScheme::Dark);
        live.process(setup);
        let mut rebuilt =
            TerminalEmulator::new(rows(rows_count), cols(cols_count), 0, ColorScheme::Dark);
        rebuilt.process(&live.repaint());
        live.process(follow_up);
        rebuilt.process(follow_up);
        assert_screens_match(&live, &rebuilt);
    }

    /// The pen a child set keeps styling what it writes after a repaint.
    #[test]
    fn repaint_keeps_the_cursor_pen() {
        assert_follow_up_matches(3, 10, b"\x1b[31mAB", b"C");
    }

    /// An erase after a repaint fills with the background the child set.
    #[test]
    fn repaint_keeps_the_erase_background() {
        assert_follow_up_matches(3, 10, b"text\x1b[1;2H\x1b[44m", b"\x1b[K");
    }

    /// A cursor saved with DECSC comes back with its position and pen.
    #[test]
    fn repaint_keeps_the_saved_cursor() {
        assert_follow_up_matches(
            4,
            10,
            b"ab\x1b[2;3H\x1b[1;32m\x1b7\x1b[0m\x1b[4;1Hzz",
            b"\x1b8Y",
        );
    }

    /// A line-drawing G0 keeps translating what the child writes.
    #[test]
    fn repaint_keeps_the_g0_charset() {
        assert_follow_up_matches(2, 10, b"x\x1b(0", b"q");
    }

    /// A line-drawing G1 shifted in with SO keeps translating.
    #[test]
    fn repaint_keeps_the_active_g1_charset() {
        assert_follow_up_matches(2, 10, b"x\x1b)0\x0e", b"q");
    }

    /// Insert mode keeps shifting the line right after a repaint.
    #[test]
    fn repaint_keeps_insert_mode() {
        assert_follow_up_matches(2, 10, b"abc\x1b[1;1H\x1b[4h", b"X");
    }

    /// Origin mode keeps addressing the cursor inside the scroll region.
    #[test]
    fn repaint_keeps_origin_mode() {
        assert_follow_up_matches(5, 10, b"\x1b[2;4r\x1b[?6h\x1b[2;2Hin", b"\x1b[1;1HZ");
    }

    /// Custom tab stops keep stopping a tab where the child set them.
    #[test]
    fn repaint_keeps_custom_tab_stops() {
        assert_follow_up_matches(2, 20, b"\x1b[3g\x1b[1;5H\x1bH\x1b[1;1H", b"\tX");
    }

    /// The primary screen behind the alternate one follows a resize made
    /// there, so leaving the alternate screen shows the same reflowed text.
    #[test]
    fn repaint_follows_a_resize_on_the_alternate_screen() {
        let mut live = TerminalEmulator::new(rows(5), cols(20), 0, ColorScheme::Dark);
        live.process(b"first line here\r\nsecond line here\r\nthird\x1b[?1049hfull");
        live.set_size(rows(3), cols(10));
        let mut rebuilt = TerminalEmulator::new(rows(3), cols(10), 0, ColorScheme::Dark);
        rebuilt.process(&live.repaint());
        live.process(b"\x1b[?1049l");
        rebuilt.process(b"\x1b[?1049l");
        assert_screens_match(&live, &rebuilt);
    }

    /// The emulator moves between threads.
    #[test]
    fn emulator_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<TerminalEmulator>();
    }

    /// A child querying the background color (OSC 11) must get an answer at all, or
    /// an agent that themes off it (Codex) renders invisible text. Regression for
    /// the migration leaving `Event::ColorRequest` unanswered.
    #[test]
    fn a_background_color_query_is_answered() {
        let mut emulator = TerminalEmulator::new(rows(10), cols(40), 0, ColorScheme::Dark);
        emulator.process(b"\x1b]11;?\x1b\\");
        let reply = String::from_utf8(emulator.take_pty_writes()).expect("utf8 color reply");
        assert!(
            reply.starts_with("\x1b]11;rgb:"),
            "the background query is answered, got {reply:?}"
        );
    }

    /// `screen_text` returns the live written lines, trimmed of trailing blanks.
    #[test]
    fn screen_text_returns_written_lines() {
        let mut emulator = TerminalEmulator::new(rows(6), cols(20), 100, ColorScheme::Dark);
        emulator.process(b"hello\r\nworld\r\n");
        let text = emulator.screen_text();
        let lines: Vec<&str> = text.lines().filter(|line| !line.is_empty()).collect();
        assert_eq!(lines, vec!["hello", "world"]);
    }

    /// A multi-megabyte OSC that is never terminated in time is dropped by
    /// the emulator's own parser too: the text around it lands and a later
    /// title still applies, fed in chunks as a PTY delivers it.
    #[test]
    fn an_endless_osc_is_dropped_and_the_screen_carries_on() {
        let mut emulator = TerminalEmulator::new(rows(4), cols(20), 0, ColorScheme::Dark);
        let flood = vec![b'x'; 4 * MAX_PENDING_BYTES];

        emulator.process(b"left\x1b]0;");
        for chunk in flood.chunks(4096) {
            emulator.process(chunk);
        }
        emulator.process(b"\x07right");

        let text = emulator.screen_text();
        assert_eq!(text.lines().next(), Some("leftright"));
    }

    /// The background query reports the configured scheme's background.
    #[test]
    fn a_background_query_follows_the_color_scheme() {
        let background = NamedColor::Background as usize;
        assert_eq!(
            resolve_color(background, ColorScheme::Dark),
            DARK_BACKGROUND
        );
        assert_eq!(
            resolve_color(background, ColorScheme::Light),
            LIGHT_BACKGROUND
        );
    }

    /// The named default foreground and background must resolve to the host
    /// default, not a palette index: a `u8` cast once wrapped 257 to 1 and painted
    /// every default-background cell red.
    #[test]
    fn the_default_colors_resolve_to_the_host_default() {
        assert_eq!(
            to_color(VtColor::Named(NamedColor::Background)),
            CellColor::Default
        );
        assert_eq!(
            to_color(VtColor::Named(NamedColor::Foreground)),
            CellColor::Default
        );
    }

    /// The 16 ANSI names and 256-color indices keep their palette slot so the host
    /// terminal applies its own theme.
    #[test]
    fn palette_colors_keep_their_index() {
        assert_eq!(
            to_color(VtColor::Named(NamedColor::Red)),
            CellColor::Indexed(1)
        );
        assert_eq!(
            to_color(VtColor::Named(NamedColor::BrightWhite)),
            CellColor::Indexed(15)
        );
        assert_eq!(to_color(VtColor::Indexed(200)), CellColor::Indexed(200));
    }

    /// True-color specifications pass through as RGB.
    #[test]
    fn true_color_specs_become_rgb() {
        assert_eq!(
            to_color(VtColor::Spec(Rgb {
                r: 10,
                g: 20,
                b: 30
            })),
            CellColor::Rgb(10, 20, 30)
        );
    }
}
