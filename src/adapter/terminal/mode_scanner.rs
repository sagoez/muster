//! Scans a child's output for the few sequences the emulator cannot report back:
//! entering or leaving the alternate screen (to snapshot the primary screen
//! first), setting the scroll region, tab stops, the active charset, and a full
//! reset.

use std::fmt;

use super::{
    bounded_parser::BoundedParser, fed::Fed, observation::Observation, scanned_modes::Modes,
};

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
