//! Watches a child's output stream for Kitty keyboard-protocol negotiation.
//!
//! A child turns on progressive enhancement by writing `CSI > flags u` (push),
//! `CSI = flags u` (set), or `CSI < number u` (pop). Standards-compliant clients
//! first probe support with `CSI ? u` and enable the protocol only after the
//! terminal answers `CSI ? flags u`; muster is that terminal for the child, so
//! the tracker both records the negotiated level and emits those query replies.
//!
//! Kitty keeps an independent keyboard stack for the primary and alternate
//! screens, so the tracker also watches the alternate-screen mode transitions
//! (`CSI ? 1049 h/l` and friends). Without this, an alternate-screen application
//! that pushes flags and exits without popping would leave the restored shell in
//! Kitty mode, and keys such as Ctrl-C would be sent as `CSI u` instead of a
//! legacy control byte.

use std::fmt;

#[cfg(test)]
use super::keyboard_protocol::KeyboardProtocol;
use super::{
    bounded_parser::BoundedParser,
    keyboard::{Keyboard, PUSH_INTRODUCER, SET_INTRODUCER, kitty_sequence},
};

/// Watches a child's output for Kitty keyboard-protocol negotiation, reporting
/// the resulting level and any query replies muster must write back to the
/// child. The primary and alternate screens keep independent state so a mode an
/// alternate-screen app leaves behind cannot leak into the restored shell. A
/// new tracker starts on the legacy encoding, and a child's request is masked
/// to [`SUPPORTED_FLAGS`] before it takes effect.
#[derive(Default)]
pub struct KittyKeyboardTracker {
    /// The VT parser, kept across chunks so a split sequence still completes.
    parser: BoundedParser,
    /// What the parsed sequences negotiated.
    keyboard: Keyboard,
}

impl fmt::Debug for KittyKeyboardTracker {
    /// Shows the followed keyboard state; the parser's state is opaque.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KittyKeyboardTracker")
            .field("keyboard", &self.keyboard)
            .finish_non_exhaustive()
    }
}

impl KittyKeyboardTracker {
    /// Reports the protocol the child is currently using on the active screen.
    #[cfg(test)]
    pub fn protocol(&self) -> KeyboardProtocol {
        KeyboardProtocol::from_flags(self.keyboard.active().flags())
    }

    /// Scans `bytes` from the child for keyboard-protocol sequences and
    /// alternate-screen transitions, updating the tracked level and returning
    /// any bytes to write back to the child (the reply to a `CSI ? u` query).
    /// Non-matching output is ignored, and a sequence split across calls
    /// completes in a later one.
    pub fn observe(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.parser.advance(&mut self.keyboard, bytes);
        self.keyboard.take_reply()
    }

    /// The Kitty sequences that rebuild the active screen's flags and push stack
    /// in a fresh receiver: a set for the oldest level, then one push per newer
    /// level. The inactive screen is not reproduced, since reaching it means a
    /// screen switch that would disturb an already repainted grid.
    pub fn restore_sequence(&self) -> Vec<u8> {
        let mut levels = self.keyboard.active().levels();
        let mut sequence = Vec::new();
        if let Some(oldest) = levels.next() {
            sequence.extend(kitty_sequence(SET_INTRODUCER, oldest));
        }
        for level in levels {
            sequence.extend(kitty_sequence(PUSH_INTRODUCER, level));
        }
        sequence
    }
}

#[cfg(test)]
mod tests {
    use std::iter;

    use super::{super::escape::CSI, *};

    #[test]
    fn starts_in_legacy() {
        let tracker = KittyKeyboardTracker::default();
        assert_eq!(tracker.protocol(), KeyboardProtocol::Legacy);
    }

    #[test]
    fn a_push_enables_kitty() {
        let mut tracker = KittyKeyboardTracker::default();
        assert!(tracker.observe(b"\x1b[>1u").is_empty());
        assert_eq!(tracker.protocol(), KeyboardProtocol::Kitty { flags: 1 });
    }

    #[test]
    fn a_pop_restores_the_prior_level() {
        let mut tracker = KittyKeyboardTracker::default();
        tracker.observe(b"\x1b[>5u");
        tracker.observe(b"\x1b[<u");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Legacy);
    }

    #[test]
    fn a_full_reset_clears_kitty_state() {
        let mut tracker = KittyKeyboardTracker::default();
        tracker.observe(b"\x1b[>3u");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Kitty { flags: 3 });
        // RIS (`ESC c`), as sent by `reset` after a crashed Kitty TUI.
        tracker.observe(b"\x1bc");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Legacy);
    }

    #[test]
    fn a_full_reset_returns_to_the_primary_screen() {
        let mut tracker = KittyKeyboardTracker::default();
        tracker.observe(b"\x1b[?1049h");
        tracker.observe(b"\x1b[>3u");
        tracker.observe(b"\x1bc");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Legacy);
    }

    #[test]
    fn a_device_attributes_query_is_answered() {
        let mut tracker = KittyKeyboardTracker::default();
        assert_eq!(tracker.observe(b"\x1b[c"), b"\x1b[?1;2c".to_vec());
        assert_eq!(tracker.observe(b"\x1b[0c"), b"\x1b[?1;2c".to_vec());
    }

    #[test]
    fn the_crossterm_enhancement_probe_gets_both_replies() {
        // crossterm's supports_keyboard_enhancement sends the Kitty query and DA1
        // together and blocks reading the DA1 after the flags, so both must be
        // answered or the child hangs at startup.
        let mut tracker = KittyKeyboardTracker::default();
        assert_eq!(
            tracker.observe(b"\x1b[?u\x1b[c"),
            b"\x1b[?0u\x1b[?1;2c".to_vec()
        );
    }

    #[test]
    fn secondary_device_attributes_are_not_answered_as_da1() {
        let mut tracker = KittyKeyboardTracker::default();
        assert!(tracker.observe(b"\x1b[>c").is_empty());
    }

    #[test]
    fn a_set_replaces_the_active_flags() {
        let mut tracker = KittyKeyboardTracker::default();
        tracker.observe(b"\x1b[>2u");
        tracker.observe(b"\x1b[=1u");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Kitty { flags: 1 });
    }

    #[test]
    fn a_set_in_or_mode_adds_flags() {
        let mut tracker = KittyKeyboardTracker::default();
        tracker.observe(b"\x1b[=1u");
        tracker.observe(b"\x1b[=2;2u");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Kitty { flags: 3 });
    }

    #[test]
    fn a_set_in_and_not_mode_removes_flags() {
        let mut tracker = KittyKeyboardTracker::default();
        tracker.observe(b"\x1b[=3u");
        tracker.observe(b"\x1b[=2;3u");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Kitty { flags: 1 });
    }

    #[test]
    fn ordinary_output_is_ignored() {
        let mut tracker = KittyKeyboardTracker::default();
        assert!(
            tracker
                .observe(b"hello \x1b[31mworld\x1b[0m\r\n")
                .is_empty()
        );
        assert_eq!(tracker.protocol(), KeyboardProtocol::Legacy);
    }

    #[test]
    fn a_sequence_split_across_chunks_is_reassembled() {
        let mut tracker = KittyKeyboardTracker::default();
        tracker.observe(b"\x1b[>1u\x1b[>5");
        tracker.observe(b"u\x1b[<");
        tracker.observe(b"u");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Kitty { flags: 1 });
    }

    /// A runaway sequence that never ends is dropped by the parser without
    /// holding the bytes, and the sequence after it is still recognized.
    #[test]
    fn a_trailing_escape_does_not_grow_unbounded() {
        let mut tracker = KittyKeyboardTracker::default();
        let mut flood = CSI.as_bytes().to_vec();
        flood.extend(iter::repeat_n(b'0', 128));
        flood.extend(iter::repeat_n(b';', 128));
        tracker.observe(&flood);
        tracker.observe(b"\x1b[>1u");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Kitty { flags: 1 });
    }

    /// A Kitty-looking sequence with a further intermediate is another
    /// control, and is not taken as a push or a query.
    #[test]
    fn a_kitty_sequence_with_an_intermediate_is_ignored() {
        let mut tracker = KittyKeyboardTracker::default();
        tracker.observe(b"\x1b[>1u");
        assert!(tracker.observe(b"\x1b[>3$u\x1b[?$u\x1b[$c").is_empty());
        assert_eq!(tracker.protocol(), KeyboardProtocol::Kitty { flags: 1 });
    }

    /// Bytes inside an OSC or DCS payload are payload, not sequences.
    #[test]
    fn string_payloads_are_never_scanned_as_sequences() {
        let mut tracker = KittyKeyboardTracker::default();
        assert!(
            tracker
                .observe(b"\x1b]2;[>1u[?u[c\x07\x1bP[>1u[c\x1b\\")
                .is_empty()
        );
        assert_eq!(tracker.protocol(), KeyboardProtocol::Legacy);
    }

    #[test]
    fn a_capability_query_is_answered_with_the_current_flags() {
        let mut tracker = KittyKeyboardTracker::default();
        tracker.observe(b"\x1b[>3u");
        assert_eq!(tracker.observe(b"\x1b[?u"), b"\x1b[?3u".to_vec());
    }

    #[test]
    fn a_query_before_any_push_reports_no_flags() {
        let mut tracker = KittyKeyboardTracker::default();
        assert_eq!(tracker.observe(b"\x1b[?u"), b"\x1b[?0u".to_vec());
    }

    #[test]
    fn unsupported_flags_are_masked_off() {
        let mut tracker = KittyKeyboardTracker::default();
        // Request disambiguate (1) plus alternate-keys (4); only 1 is honored.
        tracker.observe(b"\x1b[>5u");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Kitty { flags: 1 });
        assert_eq!(tracker.observe(b"\x1b[?u"), b"\x1b[?1u".to_vec());
    }

    #[test]
    fn alternate_screen_keeps_its_own_state() {
        let mut tracker = KittyKeyboardTracker::default();
        // An alternate-screen app enables Kitty mode and exits without popping.
        tracker.observe(b"\x1b[?1049h");
        tracker.observe(b"\x1b[>1u");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Kitty { flags: 1 });
        tracker.observe(b"\x1b[?1049l");
        // The restored primary screen never entered Kitty mode.
        assert_eq!(tracker.protocol(), KeyboardProtocol::Legacy);
    }

    #[test]
    fn the_primary_screen_survives_an_alternate_session() {
        let mut tracker = KittyKeyboardTracker::default();
        tracker.observe(b"\x1b[>2u");
        tracker.observe(b"\x1b[?1049h");
        tracker.observe(b"\x1b[>1u");
        tracker.observe(b"\x1b[?1049l");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Kitty { flags: 2 });
    }

    #[test]
    fn the_alternate_screen_stack_persists_across_sessions() {
        let mut tracker = KittyKeyboardTracker::default();
        // Enter the alternate screen, enable Kitty, then leave without popping.
        tracker.observe(b"\x1b[?1049h");
        tracker.observe(b"\x1b[>1u");
        tracker.observe(b"\x1b[?1049l");
        // The restored primary screen is unaffected.
        assert_eq!(tracker.protocol(), KeyboardProtocol::Legacy);
        // Re-entering the alternate screen restores its retained Kitty state
        // rather than starting from legacy.
        tracker.observe(b"\x1b[?1049h");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Kitty { flags: 1 });
    }

    #[test]
    fn a_full_reset_clears_the_retained_alternate_stack() {
        let mut tracker = KittyKeyboardTracker::default();
        tracker.observe(b"\x1b[?1049h");
        tracker.observe(b"\x1b[>1u");
        tracker.observe(b"\x1b[?1049l");
        tracker.observe(b"\x1bc");
        // After RIS the alternate screen no longer carries its old flags.
        tracker.observe(b"\x1b[?1049h");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Legacy);
    }

    #[test]
    fn the_restore_sequence_rebuilds_the_active_flags_and_stack() {
        let mut live = KittyKeyboardTracker::default();
        live.observe(b"\x1b[>1u\x1b[>3u");
        let mut rebuilt = KittyKeyboardTracker::default();
        assert!(rebuilt.observe(&live.restore_sequence()).is_empty());
        assert_eq!(rebuilt.protocol(), KeyboardProtocol::Kitty { flags: 3 });
        rebuilt.observe(b"\x1b[<u");
        assert_eq!(rebuilt.protocol(), KeyboardProtocol::Kitty { flags: 1 });
        rebuilt.observe(b"\x1b[<u");
        assert_eq!(rebuilt.protocol(), KeyboardProtocol::Legacy);
    }

    #[test]
    fn the_restore_sequence_targets_the_alternate_screen_when_active() {
        let mut live = KittyKeyboardTracker::default();
        live.observe(b"\x1b[?1049h\x1b[>2u");
        let mut rebuilt = KittyKeyboardTracker::default();
        rebuilt.observe(b"\x1b[?1049h");
        rebuilt.observe(&live.restore_sequence());
        assert_eq!(rebuilt.protocol(), KeyboardProtocol::Kitty { flags: 2 });
        rebuilt.observe(b"\x1b[?1049l");
        assert_eq!(rebuilt.protocol(), KeyboardProtocol::Legacy);
    }

    #[test]
    fn a_legacy_tracker_restores_to_legacy() {
        let live = KittyKeyboardTracker::default();
        let mut rebuilt = KittyKeyboardTracker::default();
        rebuilt.observe(b"\x1b[>1u");
        rebuilt.observe(&live.restore_sequence());
        assert_eq!(rebuilt.protocol(), KeyboardProtocol::Legacy);
    }

    #[test]
    fn the_alternate_mode_is_found_after_other_private_modes() {
        let mut tracker = KittyKeyboardTracker::default();
        // The alternate-screen mode trails another private mode in the sequence.
        tracker.observe(b"\x1b[?25;1049h");
        tracker.observe(b"\x1b[>1u");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Kitty { flags: 1 });
        tracker.observe(b"\x1b[?25;1049l");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Legacy);
    }
}
