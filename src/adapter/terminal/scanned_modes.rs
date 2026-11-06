use alacritty_terminal::vte::{Params, Perform};
use getset::CopyGetters;

use super::{
    escape::{
        ALT_SCREEN_MODES, CAN, ESC, FULL_RESET_FINAL, PRIVATE_MARKER, RESET_MODE_FINAL,
        SCROLL_REGION_FINAL, SET_MODE_FINAL, SUB, param_values,
    },
    observation::Observation,
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
/// Intermediate byte of DECSCUSR (`CSI Ps SP q`).
const CURSOR_STYLE_INTERMEDIATE: u8 = b' ';
/// Final of DECSCUSR.
const CURSOR_STYLE_FINAL: char = 'q';
/// DECSCUSR parameter that returns the cursor to the terminal's own style.
const DEFAULT_CURSOR_STYLE: u16 = 0;

/// The modes a [`ModeScanner`](super::mode_scanner::ModeScanner) follows,
/// and what the byte it last fed completed.
#[derive(Debug, Default, CopyGetters)]
pub(super) struct Modes {
    /// Whether origin mode is on.
    #[getset(get_copy = "pub(super)")]
    origin: bool,
    /// Origin mode as the saved cursor holds it.
    saved_origin: bool,
    /// Whether the child chose a cursor style, rather than leaving the
    /// terminal's own.
    #[getset(get_copy = "pub(super)")]
    cursor_style_set: bool,
    /// What the byte being fed completed.
    #[getset(get_copy = "pub(super)")]
    observed: Observation,
    /// Whether the bytes fed so far end inside a sequence or a character.
    #[getset(get_copy = "pub(super)")]
    in_sequence: bool,
    /// Whether the byte being fed made the parser call anything.
    called: bool,
}

impl Modes {
    /// Forgets what the last byte completed, before the next is fed.
    pub(super) fn clear_observed(&mut self) {
        self.observed = Observation::Nothing;
        self.called = false;
    }

    /// Settles whether the stream stands inside a sequence after the parser
    /// kept `byte`: an ESC opens one, and a byte that made no call is part
    /// of one or of a character.
    pub(super) fn note_kept(&mut self, byte: u8) {
        if byte == ESC || !self.called {
            self.in_sequence = true;
        }
    }

    /// Notes that the parser dropped the sequence it held and starts over.
    pub(super) fn note_dropped(&mut self) {
        self.in_sequence = false;
    }

    /// Notes a call that ends a sequence or a character, or one that leaves
    /// the parser inside a string, as `inside` says.
    fn called_with(&mut self, inside: bool) {
        self.called = true;
        self.in_sequence = inside;
    }

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
    /// Ends whatever character was in progress.
    fn print(&mut self, _character: char) {
        self.called_with(false);
    }

    /// Follows a charset shift, which acts even inside a sequence; CAN and
    /// SUB cancel the sequence in progress.
    fn execute(&mut self, byte: u8) {
        self.called = true;
        match byte {
            SHIFT_OUT => self.observed = Observation::ShiftOut,
            SHIFT_IN => self.observed = Observation::ShiftIn,
            CAN | SUB => self.in_sequence = false,
            _ => {},
        }
    }

    /// Starts a DCS string, which lasts until its end.
    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _action: char) {
        self.called_with(true);
    }

    /// Takes a byte of a DCS string, which goes on.
    fn put(&mut self, _byte: u8) {
        self.called_with(true);
    }

    /// Ends a DCS string.
    fn unhook(&mut self) {
        self.called_with(false);
    }

    /// Ends an OSC string.
    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {
        self.called_with(false);
    }

    /// Follows a full reset, a tab stop set and a cursor save or restore.
    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        self.called_with(false);
        if !intermediates.is_empty() {
            return;
        }
        match byte {
            FULL_RESET_FINAL => {
                self.origin = false;
                self.saved_origin = false;
                self.cursor_style_set = false;
                self.observed = Observation::FullReset;
            },
            SAVE_CURSOR_FINAL => self.saved_origin = self.origin,
            RESTORE_CURSOR_FINAL => self.origin = self.saved_origin,
            SET_TAB_FINAL => self.observed = Observation::SetTabStop,
            _ => {},
        }
    }

    /// Follows private mode switches, DECSTBM, TBC and DECSCUSR; a sequence
    /// the parser had to cut short is ignored, as the emulator ignores it.
    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        self.called_with(false);
        if ignore {
            return;
        }
        match (intermediates, action) {
            ([PRIVATE_MARKER], SET_MODE_FINAL) => self.private_modes(params, true),
            ([PRIVATE_MARKER], RESET_MODE_FINAL) => self.private_modes(params, false),
            ([], SCROLL_REGION_FINAL) => self.observed = Self::scroll_region(params),
            ([CURSOR_STYLE_INTERMEDIATE], CURSOR_STYLE_FINAL) => {
                self.cursor_style_set = param_values(params)
                    .next()
                    .is_some_and(|style| style != DEFAULT_CURSOR_STYLE);
            },
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
