//! The Kitty keyboard flags a pane's child can negotiate, which the tracker
//! records from child output.

/// Flag value denoting no enhancement.
pub const NO_FLAGS: u16 = 0;
/// Kitty flag bit: disambiguate escape codes (Shift+Enter, Ctrl+key, Escape).
pub const DISAMBIGUATE: u16 = 0b0_0001;
/// Kitty flag bit: report key event types (press, repeat, release).
pub const REPORT_EVENT_TYPES: u16 = 0b0_0010;
/// The Kitty flags muster actually honors when relaying keys. The tracker masks
/// a child's request to this set so its capability-query reply advertises only
/// what took effect. Three flags are deliberately excluded because the bytes an
/// attached client relays cannot back them: all-key reporting (bit 8) needs
/// every key reported as an escape code by the host terminal, which a legacy
/// host has already collapsed (Shift+A arrives as `A` with no Shift), so
/// advertising it would make muster emit noncanonical events; alternate-key
/// reporting (bit 4) and associated-text reporting (bit 16) would require
/// codepoints and text the encoder never produces.
pub const SUPPORTED_FLAGS: u16 = DISAMBIGUATE | REPORT_EVENT_TYPES;

/// The keyboard protocol a pane's child has negotiated, as tests read it.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardProtocol {
    /// No enhancement: keystrokes use legacy terminal encodings.
    Legacy,
    /// Kitty progressive enhancement active with the given flag bits.
    Kitty { flags: u16 },
}

#[cfg(test)]
impl KeyboardProtocol {
    /// Interprets a raw Kitty flag value, treating an empty flag set as legacy.
    pub fn from_flags(flags: u16) -> Self {
        if flags == NO_FLAGS {
            Self::Legacy
        } else {
            Self::Kitty { flags }
        }
    }
}
