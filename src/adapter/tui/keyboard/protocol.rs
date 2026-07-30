//! The keyboard protocol a pane's child has negotiated. This is the shared
//! vocabulary the tracker (which updates it from child output) and the encoder
//! (which dispatches on it) both speak.

/// Flag value denoting no enhancement.
pub(super) const NO_FLAGS: u16 = 0;
/// Kitty flag bit: disambiguate escape codes (Shift+Enter, Ctrl+key, Escape).
pub(super) const DISAMBIGUATE: u16 = 0b0_0001;
/// Kitty flag bit: report key event types (press, repeat, release).
pub(super) const REPORT_EVENT_TYPES: u16 = 0b0_0010;
/// The Kitty flags muster actually honors when relaying keys. The tracker masks
/// a child's request to this set so its capability-query reply advertises only
/// what took effect. Three flags are deliberately excluded because muster's host
/// input, read through crossterm, cannot back them: all-key reporting (bit 8)
/// needs `REPORT_ALL_KEYS_AS_ESCAPE_CODES` on the host, which crossterm has
/// already collapsed away (Shift+A arrives as `Char('A')` with no Shift), so
/// advertising it would make muster emit noncanonical events; alternate-key
/// reporting (bit 4) and associated-text reporting (bit 16) would require
/// codepoints and text the encoder never produces.
pub(super) const SUPPORTED_FLAGS: u16 = DISAMBIGUATE | REPORT_EVENT_TYPES;

/// The keyboard protocol a pane's child has negotiated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardProtocol {
    /// No enhancement: keystrokes use legacy terminal encodings.
    Legacy,
    /// Kitty progressive enhancement active with the given flag bits.
    Kitty { flags: u16 },
}

impl KeyboardProtocol {
    /// Interprets a raw Kitty flag value, treating an empty flag set as legacy.
    pub(super) fn from_flags(flags: u16) -> Self {
        if flags == NO_FLAGS {
            Self::Legacy
        } else {
            Self::Kitty { flags }
        }
    }

    /// Whether the child asked for escape-code disambiguation, so keys with
    /// ambiguous legacy encodings (Ctrl+key, Escape, keypad) become `CSI u`.
    pub(super) fn disambiguates(self) -> bool {
        matches!(self, Self::Kitty { flags } if flags & DISAMBIGUATE != 0)
    }

    /// Whether the child asked for key event types, so encoded sequences carry a
    /// press/repeat/release suffix and releases are forwarded.
    pub(super) fn reports_events(self) -> bool {
        matches!(self, Self::Kitty { flags } if flags & REPORT_EVENT_TYPES != 0)
    }
}
