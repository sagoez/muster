use alacritty_terminal::vte::{Params, Perform};

use super::key_press::KeyPress;
use crate::adapter::terminal::escape::{KITTY_FINAL, param_values};

/// The final of an xterm modifyOtherKeys key report.
const MODIFY_OTHER_KEYS_FINAL: char = '~';
/// The first parameter of an xterm modifyOtherKeys key report.
const MODIFY_OTHER_KEYS: u16 = 27;
/// The modifier field of a key reported with no modifiers.
const NO_MODIFIERS: u16 = 1;
/// The Kitty event type of a key release.
const RELEASE_EVENT: u16 = 3;
/// Modifier bits for Caps Lock and Num Lock, which a terminal may report
/// with any key and which do not change which key it is.
const LOCK_MODIFIERS: u16 = 64 | 128;

/// What the parser made of the last byte of a held sequence.
#[derive(Default)]
pub(super) struct Parsed {
    /// Whether the byte completed anything: a sequence, a control or text.
    pub(super) completed: bool,
    /// The key press a completed control sequence reports.
    pub(super) press: Option<KeyPress>,
}

impl Perform for Parsed {
    /// Text ends a held sequence.
    fn print(&mut self, _character: char) {
        self.completed = true;
    }

    /// A control byte ends a held sequence.
    fn execute(&mut self, _byte: u8) {
        self.completed = true;
    }

    /// A string control ends a held sequence.
    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _action: char) {
        self.completed = true;
    }

    /// A completed control sequence, and the key press it reports.
    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        self.completed = true;
        if intermediates.is_empty() && !ignore {
            self.press = key_press(params, action);
        }
    }

    /// A completed escape sequence.
    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, _byte: u8) {
        self.completed = true;
    }
}

/// The key code and held modifier bits of a key press reported by a control
/// sequence with `params` and `action`, in Kitty's form or xterm's
/// modifyOtherKeys form, with lock modifiers dropped; none for a release or
/// any other sequence.
fn key_press(params: &Params, action: char) -> Option<KeyPress> {
    let (code, modifiers) = match action {
        KITTY_FINAL => {
            let mut fields = params.iter();
            let code = *fields.next()?.first()?;
            let modifier = fields.next().unwrap_or_default();
            if modifier.get(1) == Some(&RELEASE_EVENT) {
                return None;
            }
            let modifiers = modifier.first().copied().unwrap_or_default();
            (code, modifiers.max(NO_MODIFIERS))
        },
        MODIFY_OTHER_KEYS_FINAL => match param_values(params).collect::<Vec<_>>()[..] {
            [MODIFY_OTHER_KEYS, modifiers, code] => (code, modifiers),
            _ => return None,
        },
        _ => return None,
    };
    Some((code, modifiers.checked_sub(1)? & !LOCK_MODIFIERS))
}
