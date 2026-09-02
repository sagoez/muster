//! Terminal emulation for a managed pane, backed by `alacritty_terminal`.
//!
//! This wraps an `alacritty_terminal::Term` and its VT parser behind the small,
//! screen-model surface muster's runtime and render code needs: feed bytes, read
//! the visible grid, page the scrollback, and answer the handful of mode queries
//! (alternate screen, mouse protocol, application cursor). Window title, bell, and
//! progress are decoded separately from the raw byte stream by the signal reader,
//! so they are deliberately not surfaced here.

use std::{cell::RefCell, mem, rc::Rc, sync::OnceLock};

use alacritty_terminal::{
    Term,
    event::{Event, EventListener},
    grid::{Dimensions, Scroll},
    index::{Column, Line},
    term::{Config, TermMode, cell::Flags, test::TermSize},
    vte::ansi::{self, Color as VtColor, NamedColor, Rgb},
};
use ratatui::style::{Color, Modifier, Style};

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

/// One rendered grid cell: its glyph, resolved style, and whether it is the
/// trailing half of a wide character (which the renderer skips).
#[derive(Debug, Clone)]
pub struct RenderCell {
    ch: char,
    style: Style,
    spacer: bool,
}

impl RenderCell {
    /// The glyph to draw.
    pub fn ch(&self) -> char {
        self.ch
    }

    /// The resolved ratatui style for the cell.
    pub fn style(&self) -> Style {
        self.style
    }

    /// Whether this cell is a wide-character spacer the renderer should skip so
    /// the wide glyph in the previous column keeps both cells.
    pub fn is_spacer(&self) -> bool {
        self.spacer
    }
}

/// Runtime state the VT event listener records for the owning emulator to drain:
/// bytes the terminal wants written back to the child (device-status and cursor
/// reports, primary/secondary DA, and so on).
#[derive(Default)]
struct EmulatorSink {
    pty_writes: Vec<u8>,
}

/// The `alacritty_terminal` event listener. It shares [`EmulatorSink`] with the
/// emulator through `Rc<RefCell<_>>`; the terminal is single-threaded and owned by
/// the runtime loop, so no cross-thread synchronization is involved.
#[derive(Clone)]
struct EventProxy {
    sink: Rc<RefCell<EmulatorSink>>,
}

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        match event {
            // Status and identity reports the terminal wants written back verbatim.
            Event::PtyWrite(text) => self
                .sink
                .borrow_mut()
                .pty_writes
                .extend_from_slice(text.as_bytes()),
            // A child querying a color (OSC 10/11 for the default foreground and
            // background, OSC 4 for a palette entry) must get an answer, or an agent
            // like Codex, unable to read the background, themes for a light terminal
            // and renders white on white. Reply from a standard palette with a dark
            // background, the way a real terminal does.
            Event::ColorRequest(index, format) => {
                let reply = format(color_for_index(index));
                self.sink
                    .borrow_mut()
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
    sink: Rc<RefCell<EmulatorSink>>,
}

impl TerminalEmulator {
    /// Builds an emulator for a `rows` by `cols` screen retaining `scrollback`
    /// lines of history.
    pub fn new(rows: u16, cols: u16, scrollback: usize) -> Self {
        let sink = Rc::new(RefCell::new(EmulatorSink::default()));
        let proxy = EventProxy { sink: sink.clone() };
        let config = Config {
            scrolling_history: scrollback,
            ..Config::default()
        };
        let size = TermSize::new(cols as usize, rows as usize);
        let term = Term::new(config, &size, proxy);
        Self {
            term,
            parser: ansi::Processor::new(),
            sink,
        }
    }

    /// Feeds one chunk of child output through the VT parser.
    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    /// Resizes the screen to `rows` by `cols`, reflowing history.
    pub fn set_size(&mut self, rows: u16, cols: u16) {
        self.term
            .resize(TermSize::new(cols as usize, rows as usize));
    }

    /// The screen dimensions as `(rows, cols)`.
    pub fn size(&self) -> (u16, u16) {
        (self.term.screen_lines() as u16, self.term.columns() as u16)
    }

    /// Lines currently scrolled up into history (0 at the live bottom).
    pub fn scrollback(&self) -> usize {
        self.term.grid().display_offset()
    }

    /// Scrolls the viewport to `offset` lines into history, clamped to the
    /// available range. `usize::MAX` jumps to the oldest retained line.
    pub fn set_scrollback(&mut self, offset: usize) {
        let scroll = if offset == usize::MAX {
            Scroll::Top
        } else {
            let current = self.term.grid().display_offset() as i64;
            Scroll::Delta((offset as i64 - current) as i32)
        };
        self.term.scroll_display(scroll);
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
        mem::take(&mut self.sink.borrow_mut().pty_writes)
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

    /// The rendered cell at viewport `(row, col)`.
    pub fn cell(&self, row: u16, col: u16) -> RenderCell {
        let line = Line(row as i32 - self.term.grid().display_offset() as i32);
        let cell = &self.term.grid()[line][Column(col as usize)];
        let flags = cell.flags;
        let spacer = flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER);
        RenderCell {
            ch: cell.c,
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
}

/// Resolves an `alacritty_terminal` cell's colors and attribute flags to a
/// ratatui style. Named and indexed colors pass through as palette indices so the
/// host terminal applies its own theme; only true-color specs become RGB.
fn cell_style(fg: VtColor, bg: VtColor, flags: Flags) -> Style {
    let mut style = Style::default().fg(to_color(fg)).bg(to_color(bg));
    if flags.contains(Flags::BOLD) {
        style = style.add_modifier(Modifier::BOLD);
    }
    if flags.contains(Flags::DIM) {
        style = style.add_modifier(Modifier::DIM);
    }
    if flags.contains(Flags::ITALIC) {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if flags.intersects(Flags::ALL_UNDERLINES) {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if flags.contains(Flags::INVERSE) {
        style = style.add_modifier(Modifier::REVERSED);
    }
    if flags.contains(Flags::HIDDEN) {
        style = style.add_modifier(Modifier::HIDDEN);
    }
    if flags.contains(Flags::STRIKEOUT) {
        style = style.add_modifier(Modifier::CROSSED_OUT);
    }
    style
}

/// The largest `NamedColor` discriminant that maps to a 16-color palette index.
/// The named default foreground and background sit far above this (256 and 257),
/// so they - and cursor, dim, and bright aliases - resolve to the host default;
/// comparing as a wide integer avoids a `u8` cast wrapping 257 back into the
/// palette (which painted the default background red).
const LAST_ANSI_NAMED: usize = 15;

/// Maps an `alacritty_terminal` color to a ratatui color, keeping palette indices
/// so the host terminal's theme is honored.
fn to_color(color: VtColor) -> Color {
    match color {
        VtColor::Spec(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
        VtColor::Indexed(index) => Color::Indexed(index),
        VtColor::Named(named) => {
            let index = named as usize;
            if index <= LAST_ANSI_NAMED {
                Color::Indexed(index as u8)
            } else {
                Color::Reset
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

/// Dark background reported for an OSC 11 query when the host's real background
/// could not be detected, so a child still themes for a dark terminal (the common
/// default).
const FALLBACK_BACKGROUND: Rgb = Rgb {
    r: 0x12,
    g: 0x12,
    b: 0x12,
};
/// Light foreground reported for an OSC 10 query when the host's colors are
/// unknown.
const FALLBACK_FOREGROUND: Rgb = Rgb {
    r: 0xD0,
    g: 0xD0,
    b: 0xD0,
};

/// The host terminal's default background and foreground, detected once at
/// startup. A child querying OSC 10/11 must get the *host's* colors - not a fixed
/// guess - or on a light terminal a dark-guessing reply makes agents render light
/// text that vanishes into the background (and the reverse on a dark terminal).
static HOST_COLORS: OnceLock<(Rgb, Rgb)> = OnceLock::new();

/// Records the host terminal's detected background so color queries answer with
/// the real theme. The foreground is derived to contrast, since only the
/// background is detected. Set once at startup; later calls are ignored.
pub fn set_host_background(background: (u8, u8, u8)) {
    let background = Rgb {
        r: background.0,
        g: background.1,
        b: background.2,
    };
    let foreground = if is_light(background) {
        Rgb {
            r: 0x10,
            g: 0x10,
            b: 0x10,
        }
    } else {
        Rgb {
            r: 0xE0,
            g: 0xE0,
            b: 0xE0,
        }
    };
    let _ = HOST_COLORS.set((background, foreground));
}

/// Whether `color` reads as light, by WCAG relative luminance.
fn is_light(color: Rgb) -> bool {
    let channel = |value: u8| {
        let value = f32::from(value) / 255.0;
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b) > 0.5
}

/// Resolves a color-query index against the detected host colors.
fn color_for_index(index: usize) -> Rgb {
    resolve_color(index, HOST_COLORS.get().copied())
}

/// Resolves a color-query index to an RGB value: the xterm 256-color palette for
/// indices 0-255, and `host` background/foreground (or a dark fallback) for the
/// named default background/foreground.
fn resolve_color(index: usize, host: Option<(Rgb, Rgb)>) -> Rgb {
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
        i if i == NamedColor::Background as usize => host.map_or(FALLBACK_BACKGROUND, |(bg, _)| bg),
        _ => host.map_or(FALLBACK_FOREGROUND, |(_, fg)| fg),
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
    use alacritty_terminal::vte::ansi::{Color as VtColor, NamedColor, Rgb};
    use ratatui::style::Color;

    use super::{FALLBACK_BACKGROUND, TerminalEmulator, resolve_color, to_color};

    /// A child querying the background color (OSC 11) must get an answer at all, or
    /// an agent that themes off it (Codex) renders invisible text. Regression for
    /// the migration leaving `Event::ColorRequest` unanswered.
    #[test]
    fn a_background_color_query_is_answered() {
        let mut emulator = TerminalEmulator::new(10, 40, 0);
        emulator.process(b"\x1b]11;?\x1b\\");
        let reply = String::from_utf8(emulator.take_pty_writes()).expect("utf8 color reply");
        assert!(
            reply.starts_with("\x1b]11;rgb:"),
            "the background query is answered, got {reply:?}"
        );
    }

    /// The reported background is the detected host color, so an agent themes for
    /// the real terminal - a light host reports light, not a dark guess.
    #[test]
    fn a_color_query_reports_the_host_background() {
        let light = Rgb {
            r: 0xF5,
            g: 0xF5,
            b: 0xF0,
        };
        let dark_fg = Rgb {
            r: 0x10,
            g: 0x10,
            b: 0x10,
        };
        let background = NamedColor::Background as usize;
        assert_eq!(
            resolve_color(background, Some((light, dark_fg))),
            light,
            "a light host is reported light, not a dark fallback"
        );
    }

    /// Without a detected host, the background falls back to dark - the common
    /// terminal default.
    #[test]
    fn a_color_query_falls_back_dark_without_a_host() {
        let background = NamedColor::Background as usize;
        assert_eq!(resolve_color(background, None), FALLBACK_BACKGROUND);
    }

    /// The named default foreground and background must resolve to the host
    /// default, not a palette index: a `u8` cast once wrapped 257 to 1 and painted
    /// every default-background cell red.
    #[test]
    fn the_default_colors_resolve_to_the_host_default() {
        assert_eq!(
            to_color(VtColor::Named(NamedColor::Background)),
            Color::Reset
        );
        assert_eq!(
            to_color(VtColor::Named(NamedColor::Foreground)),
            Color::Reset
        );
    }

    /// The 16 ANSI names and 256-color indices keep their palette slot so the host
    /// terminal applies its own theme.
    #[test]
    fn palette_colors_keep_their_index() {
        assert_eq!(to_color(VtColor::Named(NamedColor::Red)), Color::Indexed(1));
        assert_eq!(
            to_color(VtColor::Named(NamedColor::BrightWhite)),
            Color::Indexed(15)
        );
        assert_eq!(to_color(VtColor::Indexed(200)), Color::Indexed(200));
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
            Color::Rgb(10, 20, 30)
        );
    }
}
