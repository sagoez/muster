//! Scans a child's output for the few sequences the emulator cannot report back:
//! entering or leaving the alternate screen (to snapshot the primary screen
//! first), setting the scroll region, tab stops, the active charset, and a full
//! reset.

/// Escape byte that introduces a control sequence.
const ESC: u8 = 0x1B;
/// Shift Out: makes G1 the active charset, even inside a sequence.
const SHIFT_OUT: u8 = 0x0E;
/// Shift In: makes G0 the active charset, even inside a sequence.
const SHIFT_IN: u8 = 0x0F;
/// Final byte of HTS (`ESC H`), which sets a tab stop at the cursor.
const SET_TAB_FINAL: u8 = b'H';
/// Final byte of TBC (`CSI g`), which clears tab stops.
const CLEAR_TABS_FINAL: u8 = b'g';
/// TBC parameter clearing the stop at the cursor.
const CLEAR_CURRENT_TAB: u16 = 0;
/// TBC parameter clearing every stop.
const CLEAR_ALL_TABS: u16 = 3;
/// Cancel control byte, which aborts a sequence in progress.
const CAN: u8 = 0x18;
/// Substitute control byte, which aborts a sequence in progress.
const SUB: u8 = 0x1A;
/// Second byte of the Control Sequence Introducer.
const CSI_INTRODUCER: u8 = b'[';
/// Final byte of a full terminal reset (`ESC c`).
const RESET_FINAL: u8 = b'c';
/// Marker byte of a DEC private sequence.
const PRIVATE_MARKER: u8 = b'?';
/// Separator between CSI parameters.
const PARAM_SEPARATOR: u8 = b';';
/// Final byte that sets a private mode.
const SET_MODE_FINAL: u8 = b'h';
/// Final byte that resets a private mode.
const RESET_MODE_FINAL: u8 = b'l';
/// Final byte that sets the scroll region (DECSTBM).
const SCROLL_REGION_FINAL: u8 = b'r';
/// Lowest intermediate byte of a CSI sequence.
const INTERMEDIATE_MIN: u8 = 0x20;
/// Highest intermediate byte of a CSI sequence.
const INTERMEDIATE_MAX: u8 = 0x2F;
/// Lowest final byte of a CSI sequence.
const FINAL_MIN: u8 = 0x40;
/// Highest final byte of a CSI sequence.
const FINAL_MAX: u8 = 0x7E;
/// The private mode that confines cursor addressing to the scroll region.
const ORIGIN_MODE: u16 = 6;
/// The alternate-screen mode that also saves and restores the cursor.
const SAVING_ALT_SCREEN_MODE: u16 = 1049;
/// Final byte of DECSC (`ESC 7`), which saves the cursor.
const SAVE_CURSOR_FINAL: u8 = b'7';
/// Final byte of DECRC (`ESC 8`), which restores it.
const RESTORE_CURSOR_FINAL: u8 = b'8';
/// Private modes that switch to the alternate screen.
const ALT_SCREEN_MODES: [u16; 3] = [47, 1047, 1049];
/// Most parameters retained for one sequence.
const MAX_PARAMS: usize = 16;
/// Top margin assumed when DECSTBM omits it.
const DEFAULT_TOP_MARGIN: usize = 1;

/// What the scanner concluded after consuming one byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Observation {
    /// Nothing the emulator needs to act on.
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
}

/// Where the scanner is within an escape sequence.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum State {
    #[default]
    Ground,
    Escape,
    Csi,
}

/// Byte-at-a-time recognizer for the sequences in [`Observation`]; its state
/// carries across output chunks so a split sequence is still recognized. It
/// also follows origin mode, which a saved cursor carries: `ESC 7` and
/// entering the alternate screen with 1049 save it, `ESC 8` and leaving with
/// 1049 restore it.
#[derive(Debug, Default)]
pub struct ModeScanner {
    state: State,
    private: bool,
    params: Vec<u16>,
    current: Option<u16>,
    /// Whether origin mode is on.
    origin: bool,
    /// Origin mode as the saved cursor holds it.
    saved_origin: bool,
}

impl ModeScanner {
    /// Creates a scanner at the start of a stream.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether origin mode (DECOM) is on after what was fed.
    pub fn origin_mode(&self) -> bool {
        self.origin
    }

    /// Consumes one output byte and reports what, if anything, it completed.
    pub fn feed(&mut self, byte: u8) -> Observation {
        match byte {
            SHIFT_OUT => return Observation::ShiftOut,
            SHIFT_IN => return Observation::ShiftIn,
            _ => {},
        }
        match self.state {
            State::Ground => {
                if byte == ESC {
                    self.state = State::Escape;
                }
                Observation::Nothing
            },
            State::Escape => self.feed_escape(byte),
            State::Csi => self.feed_csi(byte),
        }
    }

    /// Advances from the byte after `ESC`.
    fn feed_escape(&mut self, byte: u8) -> Observation {
        match byte {
            CSI_INTRODUCER => {
                self.state = State::Csi;
                self.private = false;
                self.params.clear();
                self.current = None;
                Observation::Nothing
            },
            RESET_FINAL => {
                self.state = State::Ground;
                self.origin = false;
                self.saved_origin = false;
                Observation::FullReset
            },
            SAVE_CURSOR_FINAL => {
                self.state = State::Ground;
                self.saved_origin = self.origin;
                Observation::Nothing
            },
            RESTORE_CURSOR_FINAL => {
                self.state = State::Ground;
                self.origin = self.saved_origin;
                Observation::Nothing
            },
            SET_TAB_FINAL => {
                self.state = State::Ground;
                Observation::SetTabStop
            },
            ESC => Observation::Nothing,
            _ => {
                self.state = State::Ground;
                Observation::Nothing
            },
        }
    }

    /// Advances within a CSI sequence.
    fn feed_csi(&mut self, byte: u8) -> Observation {
        match byte {
            PRIVATE_MARKER => self.private = true,
            b'0'..=b'9' => {
                let digit = u16::from(byte - b'0');
                let value = self.current.unwrap_or(0);
                self.current = Some(value.saturating_mul(10).saturating_add(digit));
            },
            PARAM_SEPARATOR => self.push_param(),
            INTERMEDIATE_MIN..=INTERMEDIATE_MAX => {},
            FINAL_MIN..=FINAL_MAX => {
                self.state = State::Ground;
                self.push_param();
                return self.finish(byte);
            },
            ESC => self.state = State::Escape,
            CAN | SUB => self.state = State::Ground,
            _ => {},
        }
        Observation::Nothing
    }

    /// Stores the parameter being accumulated, treating an empty one as zero.
    fn push_param(&mut self) {
        let value = self.current.take().unwrap_or(0);
        if self.params.len() < MAX_PARAMS {
            self.params.push(value);
        }
    }

    /// Applies the private modes of a completed set (`set`) or reset to
    /// origin mode, in order: mode 6 itself, and 1049, whose entry saves the
    /// cursor and whose exit restores it.
    fn follow_origin(&mut self, set: bool) {
        for &mode in &self.params {
            match mode {
                ORIGIN_MODE => self.origin = set,
                SAVING_ALT_SCREEN_MODE if set => self.saved_origin = self.origin,
                SAVING_ALT_SCREEN_MODE => self.origin = self.saved_origin,
                _ => {},
            }
        }
    }

    /// Interprets a completed sequence ending in `final_byte`.
    fn finish(&mut self, final_byte: u8) -> Observation {
        if self.private && matches!(final_byte, SET_MODE_FINAL | RESET_MODE_FINAL) {
            self.follow_origin(final_byte == SET_MODE_FINAL);
        }
        let alternate = self
            .params
            .iter()
            .any(|mode| ALT_SCREEN_MODES.contains(mode));
        match (self.private, final_byte) {
            (true, SET_MODE_FINAL) if alternate => Observation::EnterAlternate,
            (true, RESET_MODE_FINAL) if alternate => Observation::LeaveAlternate,
            (false, SCROLL_REGION_FINAL) => {
                let margin = |index: usize| {
                    self.params
                        .get(index)
                        .copied()
                        .filter(|value| *value != 0)
                        .map(usize::from)
                };
                Observation::ScrollRegion {
                    top: margin(0).unwrap_or(DEFAULT_TOP_MARGIN),
                    bottom: margin(1),
                }
            },
            (false, CLEAR_TABS_FINAL) => match self.params.first().copied() {
                Some(CLEAR_CURRENT_TAB) => Observation::ClearTabStop,
                Some(CLEAR_ALL_TABS) => Observation::ClearAllTabStops,
                _ => Observation::Nothing,
            },
            _ => Observation::Nothing,
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
