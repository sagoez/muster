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

use super::protocol::{KeyboardProtocol, NO_FLAGS, SUPPORTED_FLAGS};

/// Escape byte that introduces a control sequence.
const ESC: u8 = 0x1B;
/// Second byte of the Control Sequence Introducer, following `ESC`.
const CSI_INTRODUCER: u8 = b'[';
/// Second byte of a full terminal reset (RIS, `ESC c`), which clears all keyboard
/// enhancement state just as a real terminal does.
const RIS_FINAL: u8 = b'c';
/// Final byte marking a Kitty keyboard control sequence (`CSI ... u`).
const KITTY_FINAL: u8 = b'u';
/// Final byte that sets a DEC private mode (`CSI ? n h`).
const SET_MODE_FINAL: u8 = b'h';
/// Final byte that resets a DEC private mode (`CSI ? n l`).
const RESET_MODE_FINAL: u8 = b'l';
/// Final byte of a device-attributes query (`CSI c`), including its DA1 form.
const DEVICE_ATTRIBUTES_FINAL: u8 = b'c';
/// Primary device-attributes (DA1) response muster returns: a VT100 with the
/// Advanced Video Option, a conservative baseline that keeps children on
/// sequences muster's VT parser handles.
const DEVICE_ATTRIBUTES: &[u8] = b"\x1b[?1;2c";
/// Separator between CSI parameters.
const PARAM_SEPARATOR: u8 = b';';
/// Leading byte of a flag push (`CSI > flags u`).
const PUSH_INTRODUCER: u8 = b'>';
/// Leading byte of an absolute flag set (`CSI = flags u`).
const SET_INTRODUCER: u8 = b'=';
/// Leading byte of a flag pop (`CSI < number u`).
const POP_INTRODUCER: u8 = b'<';
/// Leading byte of a flag query (`CSI ? u`), and of a DEC private mode.
const PRIVATE_MARKER: u8 = b'?';
/// Lowest byte value that terminates a CSI sequence (its "final byte" range).
const CSI_FINAL_MIN: u8 = 0x40;
/// Highest byte value that terminates a CSI sequence.
const CSI_FINAL_MAX: u8 = 0x7E;
/// Largest partial CSI prefix retained between output chunks.
const MAX_PENDING_BYTES: usize = 64;
/// Pop count assumed when a `CSI < u` sequence omits its number.
const DEFAULT_POP_COUNT: u16 = 1;
/// `CSI = flags ; mode u` mode that replaces the active flags outright (default).
const SET_MODE_REPLACE: u16 = 1;
/// Set mode that adds the given flags to the active set (bitwise or).
const SET_MODE_OR: u16 = 2;
/// Set mode that removes the given flags from the active set (bitwise and-not).
const SET_MODE_AND_NOT: u16 = 3;
/// Maximum depth of the flag stack; further pushes drop the oldest entry so a
/// buggy or hostile child cannot grow muster's memory without bound. Kitty
/// itself caps the stack at this depth.
const STACK_LIMIT: usize = 16;
/// DEC private modes that switch to and from the alternate screen.
const ALT_SCREEN_MODES: [u16; 3] = [47, 1047, 1049];

/// One screen's keyboard-enhancement state: the active flags and the push stack.
#[derive(Debug, Default)]
struct ScreenState {
    /// Flag values saved by pushes, restored by pops, most recent last.
    stack: Vec<u16>,
    /// The currently active flag value.
    flags: u16,
}

/// Watches a child's output for Kitty keyboard-protocol negotiation, reporting
/// the resulting level and any query replies muster must write back to the
/// child. The primary and alternate screens keep independent state so a mode an
/// alternate-screen app leaves behind cannot leak into the restored shell.
#[derive(Debug)]
pub struct KittyKeyboardTracker {
    /// The flags muster can actually honor, derived from the host input
    /// capability; a child's request is masked to these before it takes effect.
    supported: u16,
    /// A CSI prefix split across the previous chunk boundary, awaiting more
    /// bytes before it can be parsed.
    pending: Vec<u8>,
    /// Keyboard state for the primary screen.
    primary: ScreenState,
    /// Keyboard state for the alternate screen.
    alternate: ScreenState,
    /// Whether the child is currently on the alternate screen.
    on_alternate: bool,
    /// Bumped whenever the active screen changes (alternate-screen entry/exit or a
    /// full reset). Held keys capture it so a release can be dropped when the
    /// screen changed under them even if both screens carry identical flags.
    screen_epoch: u64,
}

impl KittyKeyboardTracker {
    /// Creates a tracker whose accepted flags reflect the host input capability.
    /// When the host did not enable keyboard enhancement, crossterm delivers
    /// only legacy events, so no flags are honored and children stay on the
    /// legacy encoding muster can actually deliver.
    pub fn new(host_enhanced: bool) -> Self {
        Self {
            supported: if host_enhanced {
                SUPPORTED_FLAGS
            } else {
                NO_FLAGS
            },
            pending: Vec::new(),
            primary: ScreenState::default(),
            alternate: ScreenState::default(),
            on_alternate: false,
            screen_epoch: 0,
        }
    }

    /// Reports the protocol the child is currently using on the active screen.
    pub fn protocol(&self) -> KeyboardProtocol {
        KeyboardProtocol::from_flags(self.active().flags)
    }

    /// A counter identifying the active screen session, bumped on every
    /// alternate-screen transition and full reset. Two states with equal flags
    /// but different epochs are different screens.
    pub fn screen_epoch(&self) -> u64 {
        self.screen_epoch
    }

    /// Scans `bytes` from the child for keyboard-protocol sequences and
    /// alternate-screen transitions, updating the tracked level and returning
    /// any bytes to write back to the child (the reply to a `CSI ? u` query).
    /// Non-matching output is ignored, and a sequence split across calls is
    /// buffered until it completes.
    pub fn observe(&mut self, bytes: &[u8]) -> Vec<u8> {
        let combined;
        let bytes = if self.pending.is_empty() {
            bytes
        } else {
            combined = self
                .pending
                .iter()
                .copied()
                .chain(bytes.iter().copied())
                .collect::<Vec<_>>();
            self.pending.clear();
            &combined
        };

        let mut reply = Vec::new();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] != ESC {
                index += 1;
                continue;
            }
            let Some(&next) = bytes.get(index + 1) else {
                self.store_pending(&bytes[index..]);
                return reply;
            };
            // A full reset (RIS) clears all enhancement state, so a `reset` after a
            // crashed Kitty TUI returns the restored shell to legacy input.
            if next == RIS_FINAL {
                self.reset();
                index += 2;
                continue;
            }
            if next != CSI_INTRODUCER {
                index += 1;
                continue;
            }
            let mut end = index + 2;
            while end < bytes.len() && !(CSI_FINAL_MIN..=CSI_FINAL_MAX).contains(&bytes[end]) {
                end += 1;
            }
            if end >= bytes.len() {
                self.store_pending(&bytes[index..]);
                return reply;
            }
            let params = &bytes[index + 2..end];
            match bytes[end] {
                KITTY_FINAL => self.apply(params, &mut reply),
                SET_MODE_FINAL | RESET_MODE_FINAL => self.screen_mode(params, bytes[end]),
                DEVICE_ATTRIBUTES_FINAL => reply_device_attributes(params, &mut reply),
                _ => {},
            }
            index = end + 1;
        }
        reply
    }

    /// Clears all keyboard-enhancement state, as a full terminal reset (RIS) does:
    /// both screens return to legacy with empty stacks and the primary screen
    /// becomes active, so input after the reset is encoded as legacy again.
    fn reset(&mut self) {
        self.primary = ScreenState::default();
        self.alternate = ScreenState::default();
        self.on_alternate = false;
        self.screen_epoch = self.screen_epoch.wrapping_add(1);
    }

    /// The keyboard state of the currently active screen.
    fn active(&self) -> &ScreenState {
        if self.on_alternate {
            &self.alternate
        } else {
            &self.primary
        }
    }

    /// The mutable keyboard state of the currently active screen.
    fn active_mut(&mut self) -> &mut ScreenState {
        if self.on_alternate {
            &mut self.alternate
        } else {
            &mut self.primary
        }
    }

    /// Retains a partial CSI prefix for the next call, dropping anything longer
    /// than a plausible sequence so a lone `ESC` in binary output cannot grow an
    /// unbounded buffer.
    fn store_pending(&mut self, bytes: &[u8]) {
        self.pending.clear();
        if bytes.len() <= MAX_PENDING_BYTES {
            self.pending.extend_from_slice(bytes);
        }
    }

    /// Applies the parameters of one `CSI ... u` sequence to the active screen,
    /// appending a reply to `reply` when the sequence is a capability query.
    ///
    /// Requested flags are masked to those muster actually honors ([`SUPPORTED_FLAGS`]
    /// = disambiguate + event types). This is the behavior the Kitty protocol
    /// prescribes - the terminal applies only the flags it supports and ignores the
    /// rest - and it is not silent: the capability query below replies with the true
    /// active (masked) flags, so a child that queries after pushing learns exactly
    /// what took effect. muster deliberately does not implement report-all
    /// (`0b1000`) or alternate-keys (`0b100`); the agent CLIs it runs use only
    /// disambiguate + event types, and honoring report-all would mean re-encoding
    /// all literal text as `CSI u`. Do not widen `supported` to advertise flags the
    /// encoder does not produce.
    fn apply(&mut self, params: &[u8], reply: &mut Vec<u8>) {
        let Some((&introducer, rest)) = params.split_first() else {
            return;
        };
        match introducer {
            PUSH_INTRODUCER => {
                let flags = parse_flags(rest) & self.supported;
                let screen = self.active_mut();
                if screen.stack.len() >= STACK_LIMIT {
                    screen.stack.remove(0);
                }
                screen.stack.push(screen.flags);
                screen.flags = flags;
            },
            SET_INTRODUCER => {
                let flags = parse_flags(rest) & self.supported;
                let mode = parse_mode(rest);
                let screen = self.active_mut();
                screen.flags = match mode {
                    SET_MODE_OR => screen.flags | flags,
                    SET_MODE_AND_NOT => screen.flags & !flags,
                    _ => flags,
                };
            },
            POP_INTRODUCER => {
                let count = parse_flags(rest).max(DEFAULT_POP_COUNT);
                let screen = self.active_mut();
                for _ in 0..count {
                    screen.flags = screen.stack.pop().unwrap_or(NO_FLAGS);
                }
            },
            // Only answer the capability query when muster can honor at least
            // one flag. If the host granted no enhancement, staying silent lets
            // the child detect no Kitty support and use legacy input rather than
            // negotiating a protocol muster would silently mask.
            PRIVATE_MARKER if self.supported != NO_FLAGS => {
                reply.extend_from_slice(format!("\x1b[?{}u", self.active().flags).as_bytes());
            },
            _ => {},
        }
    }

    /// Switches the active screen when a DEC private mode toggles the alternate
    /// screen. The primary and alternate screens keep independent, persistent
    /// keyboard stacks: entering selects the alternate screen's retained state
    /// (Kitty preserves it across sessions, so an app that leaves and re-enters
    /// without repushing keeps its negotiated protocol) rather than clearing it,
    /// while the primary state is left untouched so an alternate-screen app cannot
    /// leave the restored shell in Kitty mode. Only a full reset (RIS) clears both.
    fn screen_mode(&mut self, params: &[u8], final_byte: u8) {
        let Some((&PRIVATE_MARKER, rest)) = params.split_first() else {
            return;
        };
        // A single sequence may set several private modes (`CSI ?25;1049h`), so
        // every semicolon-separated parameter is inspected, not just the first.
        let toggles_alternate = rest
            .split(|byte| *byte == PARAM_SEPARATOR)
            .filter_map(|param| std::str::from_utf8(param).ok()?.parse::<u16>().ok())
            .any(|mode| ALT_SCREEN_MODES.contains(&mode));
        if !toggles_alternate {
            return;
        }
        match final_byte {
            SET_MODE_FINAL if !self.on_alternate => {
                self.on_alternate = true;
                self.screen_epoch = self.screen_epoch.wrapping_add(1);
            },
            RESET_MODE_FINAL if self.on_alternate => {
                self.on_alternate = false;
                self.screen_epoch = self.screen_epoch.wrapping_add(1);
            },
            _ => {},
        }
    }
}

/// Answers a primary device-attributes query (`CSI c` or `CSI 0 c`) with muster's
/// [`DEVICE_ATTRIBUTES`], ignoring the secondary (`CSI > c`) and tertiary
/// (`CSI = c`) forms. crossterm's `supports_keyboard_enhancement` probe sends this
/// alongside the Kitty query and blocks reading the reply once it has the flags, so
/// an unanswered DA1 hangs a crossterm child at startup.
fn reply_device_attributes(params: &[u8], reply: &mut Vec<u8>) {
    if params.is_empty() || params == b"0" {
        reply.extend_from_slice(DEVICE_ATTRIBUTES);
    }
}

/// Parses the leading numeric parameter of a Kitty sequence, defaulting to no
/// flags when it is absent or malformed.
fn parse_flags(bytes: &[u8]) -> u16 {
    parse_param(bytes, 0).unwrap_or(NO_FLAGS)
}

/// Parses the second parameter of a `CSI = flags ; mode u` sequence, defaulting
/// to the replace mode when it is absent or malformed.
fn parse_mode(bytes: &[u8]) -> u16 {
    parse_param(bytes, 1).unwrap_or(SET_MODE_REPLACE)
}

/// Parses the `index`-th `;`-separated numeric parameter, if present and valid.
fn parse_param(bytes: &[u8], index: usize) -> Option<u16> {
    let param = bytes.split(|byte| *byte == PARAM_SEPARATOR).nth(index)?;
    std::str::from_utf8(param).ok()?.parse::<u16>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_in_legacy() {
        let tracker = KittyKeyboardTracker::new(true);
        assert_eq!(tracker.protocol(), KeyboardProtocol::Legacy);
    }

    #[test]
    fn a_push_enables_kitty() {
        let mut tracker = KittyKeyboardTracker::new(true);
        assert!(tracker.observe(b"\x1b[>1u").is_empty());
        assert_eq!(tracker.protocol(), KeyboardProtocol::Kitty { flags: 1 });
    }

    #[test]
    fn a_pop_restores_the_prior_level() {
        let mut tracker = KittyKeyboardTracker::new(true);
        tracker.observe(b"\x1b[>5u");
        tracker.observe(b"\x1b[<u");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Legacy);
    }

    #[test]
    fn a_full_reset_clears_kitty_state() {
        let mut tracker = KittyKeyboardTracker::new(true);
        tracker.observe(b"\x1b[>3u");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Kitty { flags: 3 });
        // RIS (`ESC c`), as sent by `reset` after a crashed Kitty TUI.
        tracker.observe(b"\x1bc");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Legacy);
    }

    #[test]
    fn a_full_reset_returns_to_the_primary_screen() {
        let mut tracker = KittyKeyboardTracker::new(true);
        tracker.observe(b"\x1b[?1049h");
        tracker.observe(b"\x1b[>3u");
        tracker.observe(b"\x1bc");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Legacy);
    }

    #[test]
    fn a_device_attributes_query_is_answered() {
        let mut tracker = KittyKeyboardTracker::new(true);
        assert_eq!(tracker.observe(b"\x1b[c"), b"\x1b[?1;2c".to_vec());
        assert_eq!(tracker.observe(b"\x1b[0c"), b"\x1b[?1;2c".to_vec());
    }

    #[test]
    fn the_crossterm_enhancement_probe_gets_both_replies() {
        // crossterm's supports_keyboard_enhancement sends the Kitty query and DA1
        // together and blocks reading the DA1 after the flags, so both must be
        // answered or the child hangs at startup.
        let mut tracker = KittyKeyboardTracker::new(true);
        assert_eq!(
            tracker.observe(b"\x1b[?u\x1b[c"),
            b"\x1b[?0u\x1b[?1;2c".to_vec()
        );
    }

    #[test]
    fn secondary_device_attributes_are_not_answered_as_da1() {
        let mut tracker = KittyKeyboardTracker::new(true);
        assert!(tracker.observe(b"\x1b[>c").is_empty());
    }

    #[test]
    fn a_set_replaces_the_active_flags() {
        let mut tracker = KittyKeyboardTracker::new(true);
        tracker.observe(b"\x1b[>2u");
        tracker.observe(b"\x1b[=1u");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Kitty { flags: 1 });
    }

    #[test]
    fn a_set_in_or_mode_adds_flags() {
        let mut tracker = KittyKeyboardTracker::new(true);
        tracker.observe(b"\x1b[=1u");
        tracker.observe(b"\x1b[=2;2u");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Kitty { flags: 3 });
    }

    #[test]
    fn a_set_in_and_not_mode_removes_flags() {
        let mut tracker = KittyKeyboardTracker::new(true);
        tracker.observe(b"\x1b[=3u");
        tracker.observe(b"\x1b[=2;3u");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Kitty { flags: 1 });
    }

    #[test]
    fn ordinary_output_is_ignored() {
        let mut tracker = KittyKeyboardTracker::new(true);
        assert!(
            tracker
                .observe(b"hello \x1b[31mworld\x1b[0m\r\n")
                .is_empty()
        );
        assert_eq!(tracker.protocol(), KeyboardProtocol::Legacy);
    }

    #[test]
    fn a_sequence_split_across_chunks_is_reassembled() {
        let mut tracker = KittyKeyboardTracker::new(true);
        tracker.observe(b"\x1b[>1u\x1b[>5");
        tracker.observe(b"u\x1b[<");
        tracker.observe(b"u");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Kitty { flags: 1 });
    }

    #[test]
    fn a_trailing_escape_does_not_grow_unbounded() {
        let mut tracker = KittyKeyboardTracker::new(true);
        let mut flood = vec![ESC, CSI_INTRODUCER];
        flood.extend(std::iter::repeat_n(b'0', MAX_PENDING_BYTES * 2));
        tracker.observe(&flood);
        assert!(tracker.pending.is_empty());
    }

    #[test]
    fn a_capability_query_is_answered_with_the_current_flags() {
        let mut tracker = KittyKeyboardTracker::new(true);
        tracker.observe(b"\x1b[>3u");
        assert_eq!(tracker.observe(b"\x1b[?u"), b"\x1b[?3u".to_vec());
    }

    #[test]
    fn a_query_before_any_push_reports_no_flags() {
        let mut tracker = KittyKeyboardTracker::new(true);
        assert_eq!(tracker.observe(b"\x1b[?u"), b"\x1b[?0u".to_vec());
    }

    #[test]
    fn unbalanced_pushes_stay_bounded() {
        let mut tracker = KittyKeyboardTracker::new(true);
        for _ in 0..(STACK_LIMIT * 4) {
            tracker.observe(b"\x1b[>1u");
        }
        assert!(tracker.primary.stack.len() <= STACK_LIMIT);
    }

    #[test]
    fn unsupported_flags_are_masked_off() {
        let mut tracker = KittyKeyboardTracker::new(true);
        // Request disambiguate (1) plus alternate-keys (4); only 1 is honored.
        tracker.observe(b"\x1b[>5u");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Kitty { flags: 1 });
        assert_eq!(tracker.observe(b"\x1b[?u"), b"\x1b[?1u".to_vec());
    }

    #[test]
    fn alternate_screen_keeps_its_own_state() {
        let mut tracker = KittyKeyboardTracker::new(true);
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
        let mut tracker = KittyKeyboardTracker::new(true);
        tracker.observe(b"\x1b[>2u");
        tracker.observe(b"\x1b[?1049h");
        tracker.observe(b"\x1b[>1u");
        tracker.observe(b"\x1b[?1049l");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Kitty { flags: 2 });
    }

    #[test]
    fn the_alternate_screen_stack_persists_across_sessions() {
        let mut tracker = KittyKeyboardTracker::new(true);
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
        let mut tracker = KittyKeyboardTracker::new(true);
        tracker.observe(b"\x1b[?1049h");
        tracker.observe(b"\x1b[>1u");
        tracker.observe(b"\x1b[?1049l");
        tracker.observe(b"\x1bc");
        // After RIS the alternate screen no longer carries its old flags.
        tracker.observe(b"\x1b[?1049h");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Legacy);
    }

    #[test]
    fn a_non_enhanced_host_honors_no_flags() {
        let mut tracker = KittyKeyboardTracker::new(false);
        // The child requests disambiguation and event types, but the host cannot
        // deliver them, so nothing is honored and the capability query goes
        // unanswered rather than advertising a protocol muster would only mask.
        tracker.observe(b"\x1b[>3u");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Legacy);
        assert!(tracker.observe(b"\x1b[?u").is_empty());
    }

    #[test]
    fn the_alternate_mode_is_found_after_other_private_modes() {
        let mut tracker = KittyKeyboardTracker::new(true);
        // The alternate-screen mode trails another private mode in the sequence.
        tracker.observe(b"\x1b[?25;1049h");
        tracker.observe(b"\x1b[>1u");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Kitty { flags: 1 });
        tracker.observe(b"\x1b[?25;1049l");
        assert_eq!(tracker.protocol(), KeyboardProtocol::Legacy);
    }
}
