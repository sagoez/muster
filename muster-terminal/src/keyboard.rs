use std::mem;

use alacritty_terminal::vte::{Params, Perform};

use super::{
    escape::{
        ALT_SCREEN_MODES, CSI, FULL_RESET_FINAL, KITTY_FINAL, PRIVATE_MARKER, RESET_MODE_FINAL,
        SET_MODE_FINAL, param_values,
    },
    keyboard_protocol::{NO_FLAGS, SUPPORTED_FLAGS},
    screen_state::ScreenState,
};

/// Final of a device-attributes query (`CSI c`), including its DA1 form.
const DEVICE_ATTRIBUTES_FINAL: char = 'c';
/// The only parameter a primary device-attributes query carries, explicit or
/// omitted.
const PRIMARY_DEVICE_ATTRIBUTES: u16 = 0;
/// Primary device-attributes (DA1) response muster returns: a VT100 with the
/// Advanced Video Option, a conservative baseline that keeps children on
/// sequences muster's VT parser handles.
const DEVICE_ATTRIBUTES: &[u8] = b"\x1b[?1;2c";
/// Intermediate of a flag push (`CSI > flags u`).
pub(super) const PUSH_INTRODUCER: u8 = b'>';
/// Intermediate of an absolute flag set (`CSI = flags u`).
pub(super) const SET_INTRODUCER: u8 = b'=';
/// Intermediate of a flag pop (`CSI < number u`).
const POP_INTRODUCER: u8 = b'<';
/// Pop count assumed when a `CSI < u` sequence omits its number.
const DEFAULT_POP_COUNT: u16 = 1;
/// `CSI = flags ; mode u` mode that replaces the active flags outright (default).
const SET_MODE_REPLACE: u16 = 1;
/// Set mode that adds the given flags to the active set (bitwise or).
const SET_MODE_OR: u16 = 2;
/// Set mode that removes the given flags from the active set (bitwise and-not).
const SET_MODE_AND_NOT: u16 = 3;

/// The keyboard state a [`super::KittyKeyboardTracker`] follows, and the
/// replies the chunk being parsed owes the child.
#[derive(Debug, Default)]
pub(super) struct Keyboard {
    /// Keyboard state for the primary screen.
    primary: ScreenState,
    /// Keyboard state for the alternate screen.
    alternate: ScreenState,
    /// Whether the child is currently on the alternate screen.
    on_alternate: bool,
    /// Bytes to write back to the child, in the order its queries came.
    reply: Vec<u8>,
}

impl Keyboard {
    /// The replies owed so far, leaving none behind.
    pub(super) fn take_reply(&mut self) -> Vec<u8> {
        mem::take(&mut self.reply)
    }

    /// Clears all keyboard-enhancement state, as a full terminal reset (RIS) does:
    /// both screens return to legacy with empty stacks and the primary screen
    /// becomes active, so input after the reset is encoded as legacy again.
    fn reset(&mut self) {
        self.primary = ScreenState::default();
        self.alternate = ScreenState::default();
        self.on_alternate = false;
    }

    /// The keyboard state of the currently active screen.
    pub(super) fn active(&self) -> &ScreenState {
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

    /// Applies one `CSI <introducer> ... u` sequence to the active screen,
    /// replying when it is a capability query.
    ///
    /// Requested flags are masked to those muster actually honors ([`SUPPORTED_FLAGS`]
    /// = disambiguate + event types). This is the behavior the Kitty protocol
    /// prescribes - the terminal applies only the flags it supports and ignores the
    /// rest - and it is not silent: the capability query below replies with the true
    /// active (masked) flags, so a child that queries after pushing learns exactly
    /// what took effect. muster deliberately does not implement report-all
    /// (`0b1000`) or alternate-keys (`0b100`); the agent CLIs it runs use only
    /// disambiguate + event types, and honoring report-all would mean re-encoding
    /// all literal text as `CSI u`. Do not widen [`SUPPORTED_FLAGS`] to advertise
    /// flags the encoder does not produce.
    fn apply(&mut self, introducer: u8, params: &Params) {
        let first = param_values(params).next().unwrap_or(NO_FLAGS);
        match introducer {
            PUSH_INTRODUCER => self.active_mut().push(first & SUPPORTED_FLAGS),
            SET_INTRODUCER => {
                let flags = first & SUPPORTED_FLAGS;
                let mode = param_values(params).nth(1).unwrap_or(SET_MODE_REPLACE);
                let screen = self.active_mut();
                let active = screen.flags();
                screen.set(match mode {
                    SET_MODE_OR => active | flags,
                    SET_MODE_AND_NOT => active & !flags,
                    _ => flags,
                });
            },
            POP_INTRODUCER => self.active_mut().pop(first.max(DEFAULT_POP_COUNT)),
            PRIVATE_MARKER => {
                let flags = self.active().flags();
                self.reply.extend(kitty_sequence(PRIVATE_MARKER, flags));
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
    /// A single sequence may set several private modes (`CSI ?25;1049h`), so
    /// every parameter is inspected.
    fn screen_mode(&mut self, params: &Params, set: bool) {
        if param_values(params).any(|mode| ALT_SCREEN_MODES.contains(&mode)) {
            self.on_alternate = set;
        }
    }

    /// Answers a primary device-attributes query (`CSI c` or `CSI 0 c`) with
    /// muster's [`DEVICE_ATTRIBUTES`]. crossterm's `supports_keyboard_enhancement`
    /// probe sends this alongside the Kitty query and blocks reading the reply
    /// once it has the flags, so an unanswered DA1 hangs a crossterm child at
    /// startup.
    fn device_attributes(&mut self, params: &Params) {
        if params.len() == 1 && param_values(params).next() == Some(PRIMARY_DEVICE_ATTRIBUTES) {
            self.reply.extend_from_slice(DEVICE_ATTRIBUTES);
        }
    }
}

impl Perform for Keyboard {
    /// A full reset (RIS) clears all enhancement state, so a `reset` after a
    /// crashed Kitty TUI returns the restored shell to legacy input.
    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        if intermediates.is_empty() && byte == FULL_RESET_FINAL {
            self.reset();
        }
    }

    /// Follows the Kitty sequences, alternate-screen switches and DA1 queries;
    /// the secondary (`CSI > c`) and tertiary (`CSI = c`) forms, any sequence
    /// with a further intermediate, and one the parser cut short are ignored.
    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        if ignore {
            return;
        }
        match (intermediates, action) {
            (&[introducer], KITTY_FINAL) => self.apply(introducer, params),
            ([PRIVATE_MARKER], SET_MODE_FINAL) => self.screen_mode(params, true),
            ([PRIVATE_MARKER], RESET_MODE_FINAL) => self.screen_mode(params, false),
            ([], DEVICE_ATTRIBUTES_FINAL) => self.device_attributes(params),
            _ => {},
        }
    }
}

/// Encodes one Kitty keyboard sequence, `CSI <introducer> <flags> u`.
pub(super) fn kitty_sequence(introducer: u8, flags: u16) -> Vec<u8> {
    format!("{CSI}{}{flags}{KITTY_FINAL}", char::from(introducer)).into_bytes()
}
