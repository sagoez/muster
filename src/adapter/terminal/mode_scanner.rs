//! Scans a child's output for the few sequences the emulator cannot report back:
//! entering or leaving the alternate screen (to snapshot the primary screen
//! first), setting the scroll region, tab stops, the active charset, and a full
//! reset.

use std::fmt;

use alacritty_terminal::vte::{Params, Perform};

use super::{
    bounded_parser::BoundedParser,
    escape::{
        ALT_SCREEN_MODES, FULL_RESET_FINAL, PRIVATE_MARKER, RESET_MODE_FINAL, SCROLL_REGION_FINAL,
        SET_MODE_FINAL, param_values,
    },
    fed::Fed,
};

/// Shift Out: makes G1 the active charset, even inside a sequence.
const SHIFT_OUT: u8 = 0x0E;
/// Shift In: makes G0 the active charset, even inside a sequence.
const SHIFT_IN: u8 = 0x0F;
/// Final byte of HTS (`ESC H`), which sets a tab stop at the cursor.
const SET_TAB_FINAL: u8 = b'H';
/// Final of TBC (`CSI g`), which clears tab stops.
const CLEAR_TABS_FINAL: char = 'g';
/// TBC parameter clearing the stop at the cursor.
const CLEAR_CURRENT_TAB: u16 = 0;
/// TBC parameter clearing every stop.
const CLEAR_ALL_TABS: u16 = 3;
/// The private mode that confines cursor addressing to the scroll region.
const ORIGIN_MODE: u16 = 6;
/// The alternate-screen mode that also saves and restores the cursor.
const SAVING_ALT_SCREEN_MODE: u16 = 1049;
/// Final byte of DECSC (`ESC 7`), which saves the cursor.
const SAVE_CURSOR_FINAL: u8 = b'7';
/// Final byte of DECRC (`ESC 8`), which restores it.
const RESTORE_CURSOR_FINAL: u8 = b'8';
/// Top margin assumed when DECSTBM omits it.
const DEFAULT_TOP_MARGIN: usize = 1;

/// What the scanner concluded after consuming one byte.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Observation {
    /// Nothing the emulator needs to act on.
    #[default]
    Nothing,
    /// This byte completes a switch to the alternate screen.
    EnterAlternate,
    /// This byte completes a switch back from the alternate screen.
    LeaveAlternate,
    /// This byte completes a scroll-region assignment (1-based margins).
    ScrollRegion {
        /// Top margin, at least 1.
        top: usize,
        /// Bottom margin, or `None` for the last screen line.
        bottom: Option<usize>,
    },
    /// This byte completes a full terminal reset.
    FullReset,
    /// This byte sets a tab stop at the cursor.
    SetTabStop,
    /// This byte clears the tab stop at the cursor.
    ClearTabStop,
    /// This byte clears every tab stop.
    ClearAllTabStops,
    /// This byte makes G1 the active charset.
    ShiftOut,
    /// This byte makes G0 the active charset.
    ShiftIn,
    /// This byte belongs to a sequence that ran past
    /// [`MAX_PENDING_BYTES`](super::bounded_parser::MAX_PENDING_BYTES), which
    /// is dropped up to its terminator; a parser fed the same stream drops it
    /// too.
    Dropped,
}

/// Byte-at-a-time recognizer for the sequences in [`Observation`], built on
/// the same VT parser as the emulator so a split sequence is still recognized
/// and string payloads are never read as sequences. It also follows origin
/// mode, which a saved cursor carries: `ESC 7` and entering the alternate
/// screen with 1049 save it, `ESC 8` and leaving with 1049 restore it.
#[derive(Default)]
pub struct ModeScanner {
    /// The VT parser, kept across bytes.
    parser: BoundedParser,
    /// What the parsed sequences established.
    modes: Modes,
}

/// The modes a [`ModeScanner`] follows, and what the byte it last fed
/// completed.
#[derive(Debug, Default)]
struct Modes {
    /// Whether origin mode is on.
    origin: bool,
    /// Origin mode as the saved cursor holds it.
    saved_origin: bool,
    /// What the byte being fed completed.
    observed: Observation,
}

impl fmt::Debug for ModeScanner {
    /// Shows the followed modes; the parser's state is opaque.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ModeScanner")
            .field("modes", &self.modes)
            .finish_non_exhaustive()
    }
}

impl ModeScanner {
    /// Creates a scanner at the start of a stream.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether origin mode (DECOM) is on after what was fed.
    pub fn origin_mode(&self) -> bool {
        self.modes.origin
    }

    /// Consumes one output byte and reports what, if anything, it completed.
    pub fn feed(&mut self, byte: u8) -> Observation {
        self.modes.observed = Observation::Nothing;
        match self.parser.advance_byte(&mut self.modes, byte) {
            Fed::Kept => self.modes.observed,
            Fed::Dropped => Observation::Dropped,
        }
    }
}

impl Modes {
    /// Applies the private modes of a completed set (`set`) or reset: origin
    /// mode, in order, with 1049's entry saving the cursor and its exit
    /// restoring it, then any alternate-screen switch.
    fn private_modes(&mut self, params: &Params, set: bool) {
        for mode in param_values(params) {
            match mode {
                ORIGIN_MODE => self.origin = set,
                SAVING_ALT_SCREEN_MODE if set => self.saved_origin = self.origin,
                SAVING_ALT_SCREEN_MODE => self.origin = self.saved_origin,
                _ => {},
            }
        }
        if param_values(params).any(|mode| ALT_SCREEN_MODES.contains(&mode)) {
            self.observed = if set {
                Observation::EnterAlternate
            } else {
                Observation::LeaveAlternate
            };
        }
    }

    /// The scroll region DECSTBM with `params` assigns.
    fn scroll_region(params: &Params) -> Observation {
        let margin = |index: usize| {
            param_values(params)
                .nth(index)
                .filter(|value| *value != 0)
                .map(usize::from)
        };
        Observation::ScrollRegion {
            top: margin(0).unwrap_or(DEFAULT_TOP_MARGIN),
            bottom: margin(1),
        }
    }
}

impl Perform for Modes {
    /// Follows a charset shift, which acts even inside a sequence.
    fn execute(&mut self, byte: u8) {
        match byte {
            SHIFT_OUT => self.observed = Observation::ShiftOut,
            SHIFT_IN => self.observed = Observation::ShiftIn,
            _ => {},
        }
    }

    /// Follows a full reset, a tab stop set and a cursor save or restore.
    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        if !intermediates.is_empty() {
            return;
        }
        match byte {
            FULL_RESET_FINAL => {
                self.origin = false;
                self.saved_origin = false;
                self.observed = Observation::FullReset;
            },
            SAVE_CURSOR_FINAL => self.saved_origin = self.origin,
            RESTORE_CURSOR_FINAL => self.origin = self.saved_origin,
            SET_TAB_FINAL => self.observed = Observation::SetTabStop,
            _ => {},
        }
    }

    /// Follows private mode switches, DECSTBM and TBC; a sequence the parser
    /// had to cut short is ignored, as the emulator ignores it.
    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        if ignore {
            return;
        }
        match (intermediates, action) {
            ([PRIVATE_MARKER], SET_MODE_FINAL) => self.private_modes(params, true),
            ([PRIVATE_MARKER], RESET_MODE_FINAL) => self.private_modes(params, false),
            ([], SCROLL_REGION_FINAL) => self.observed = Self::scroll_region(params),
            ([], CLEAR_TABS_FINAL) => {
                self.observed = match param_values(params).next() {
                    Some(CLEAR_CURRENT_TAB) => Observation::ClearTabStop,
                    Some(CLEAR_ALL_TABS) => Observation::ClearAllTabStops,
                    _ => Observation::Nothing,
                };
            },
            _ => {},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ModeScanner, Observation};

    fn scan(scanner: &mut ModeScanner, bytes: &[u8]) -> Vec<Observation> {
        bytes
            .iter()
            .map(|byte| scanner.feed(*byte))
            .filter(|observation| *observation != Observation::Nothing)
            .collect()
    }

    /// An alternate-screen switch is recognized even when split across chunks.
    #[test]
    fn an_alternate_screen_switch_is_found_across_chunks() {
        let mut scanner = ModeScanner::new();
        assert!(scan(&mut scanner, b"text\x1b[?25;10").is_empty());
        assert_eq!(scan(&mut scanner, b"49h"), vec![
            Observation::EnterAlternate
        ]);
    }

    /// Leaving the alternate screen is reported, and unrelated or public modes
    /// are ignored.
    #[test]
    fn leaving_the_alternate_screen_is_found_and_other_modes_are_ignored() {
        let mut scanner = ModeScanner::new();
        assert_eq!(scan(&mut scanner, b"\x1b[?10"), vec![]);
        assert_eq!(scan(&mut scanner, b"47l"), vec![
            Observation::LeaveAlternate
        ]);
        assert!(scan(&mut scanner, b"\x1b[?2004h\x1b[1049h\x1b[1049l\x1b[?25l").is_empty());
    }

    /// Origin mode switching on and off is followed; the public mode 6 is
    /// not origin mode.
    #[test]
    fn origin_mode_switches_are_followed() {
        let mut scanner = ModeScanner::new();
        scan(&mut scanner, b"\x1b[?6h");
        assert!(scanner.origin_mode());
        scan(&mut scanner, b"\x1b[6l");
        assert!(
            scanner.origin_mode(),
            "the public mode 6 is not origin mode"
        );
        scan(&mut scanner, b"\x1b[?6l");
        assert!(!scanner.origin_mode());
    }

    /// A combined sequence sets origin mode as well as switching screens,
    /// and the cursor saved on entering with 1049 or `ESC 7` brings back the
    /// origin mode it held.
    #[test]
    fn origin_mode_is_saved_and_restored_with_the_cursor() {
        let mut scanner = ModeScanner::new();
        assert_eq!(scan(&mut scanner, b"\x1b[?6;1049h"), vec![
            Observation::EnterAlternate
        ]);
        assert!(
            scanner.origin_mode(),
            "the combined sequence sets origin mode"
        );
        scan(&mut scanner, b"\x1b[?6l");
        assert!(!scanner.origin_mode());
        scan(&mut scanner, b"\x1b[?1049l");
        assert!(
            scanner.origin_mode(),
            "leaving restores the state saved on entry"
        );

        scan(&mut scanner, b"\x1b7\x1b[?6l");
        assert!(!scanner.origin_mode());
        scan(&mut scanner, b"\x1b8");
        assert!(
            scanner.origin_mode(),
            "DECRC restores the state DECSC saved"
        );
        scan(&mut scanner, b"\x1bc");
        assert!(!scanner.origin_mode());
    }

    /// DECSTBM yields its margins, with omitted ones left unset.
    #[test]
    fn a_scroll_region_reports_its_margins() {
        let mut scanner = ModeScanner::new();
        assert_eq!(scan(&mut scanner, b"\x1b[2;5r\x1b[r"), vec![
            Observation::ScrollRegion {
                top: 2,
                bottom: Some(5)
            },
            Observation::ScrollRegion {
                top: 1,
                bottom: None
            },
        ]);
    }

    /// A sequence carrying an intermediate is a different control: DECCARA
    /// (`CSI ... $ r`) is not DECSTBM, `CSI ... $ g` is not TBC, `ESC # H`
    /// is not HTS and `CSI ? ... $ h` switches no screen.
    #[test]
    fn a_sequence_with_an_intermediate_is_not_its_plain_namesake() {
        let mut scanner = ModeScanner::new();
        assert!(
            scan(
                &mut scanner,
                b"\x1b[1;1;2;2;7$r\x1b[3$g\x1b#H\x1b[?1049$h\x1b[?6$h"
            )
            .is_empty()
        );
        assert!(!scanner.origin_mode());
    }

    /// Bytes inside an OSC, DCS or APC payload are payload: a shift, a
    /// bracket sequence spelled out or an `ESC c` look-alike in it is not
    /// acted on, and output after the string is scanned again.
    #[test]
    fn string_payloads_are_never_scanned_as_sequences() {
        let mut scanner = ModeScanner::new();
        assert!(
            scan(
                &mut scanner,
                b"\x1b]2;a\x0eb\x0f[?1049h\x07\x1bPq\x0e[2;5r\x1b\\\x1b_G\x0e\x1b\\"
            )
            .is_empty()
        );
        assert_eq!(scan(&mut scanner, b"\x1b]0;title\x1b\\\x0e"), vec![
            Observation::ShiftOut
        ]);
    }

    /// Tab stop sets and clears are reported, and SO and SI are reported even
    /// inside a sequence, which they do not interrupt.
    #[test]
    fn tab_stops_and_charset_shifts_are_reported() {
        let mut scanner = ModeScanner::new();
        assert_eq!(scan(&mut scanner, b"\x1bH\x1b[g\x1b[3g\x1b[2g"), vec![
            Observation::SetTabStop,
            Observation::ClearTabStop,
            Observation::ClearAllTabStops,
        ]);
        assert_eq!(scan(&mut scanner, b"\x0e\x1b[?10\x0f49h"), vec![
            Observation::ShiftOut,
            Observation::ShiftIn,
            Observation::EnterAlternate,
        ]);
    }

    /// `ESC c` is a full reset, and a cancelled sequence yields nothing.
    #[test]
    fn a_reset_is_reported_and_a_cancelled_sequence_is_dropped() {
        let mut scanner = ModeScanner::new();
        assert_eq!(scan(&mut scanner, b"\x1bc"), vec![Observation::FullReset]);
        assert!(scan(&mut scanner, b"\x1b[?10\x18 49h").is_empty());
    }
}
