//! Legacy key encoding: the traditional terminal escape sequences understood by
//! every child, used for panes that have not enabled the Kitty protocol and for
//! keys the Kitty encoder defers (unmodified basic keys, cursor and function
//! keys with universal legacy modified forms).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{
    encode::{NO_MODIFIER, xterm_modifier_code},
    key::shifted_char,
};

/// Escape byte that introduces control sequences.
const ESC: u8 = 0x1B;
/// Control Sequence Introducer bytes (`ESC [`).
const CSI: &[u8] = b"\x1b[";
/// SS3 introducer bytes (`ESC O`), used by the unmodified F1-F4 sequences.
const SS3: &[u8] = b"\x1bO";
/// Carriage return sent for the Enter key.
pub(super) const CARRIAGE_RETURN: u8 = b'\r';
/// Delete byte sent for Backspace.
pub(super) const BACKSPACE_BYTE: u8 = 0x7F;
/// Horizontal tab byte.
const TAB_BYTE: u8 = b'\t';
/// Mask that folds an ASCII key to its control byte.
const CONTROL_MASK: u8 = 0x1F;
/// Final byte of the tilde-style CSI sequences (`ESC [ n ~`).
const TILDE: u8 = b'~';
/// Final SS3 byte for F1; F2-F4 follow consecutively (P, Q, R, S).
const SS3_F1: u8 = b'P';
/// Lowest function-key number.
const FIRST_FUNCTION_KEY: u8 = 1;
/// Count of function keys encoded via SS3 (F1-F4).
const SS3_FUNCTION_KEYS: u8 = 4;
/// CSI numeric parameters for F5 through F12, in order.
const CSI_FUNCTION_PARAMS: [u8; 8] = [15, 17, 18, 19, 20, 21, 23, 24];
/// Control byte for Ctrl-@ and Ctrl-Space (NUL).
const NUL: u8 = 0x00;
/// Control byte for Ctrl-? (DEL), which is not the 0x1f-masked value.
const CTRL_QUESTION: u8 = 0x7F;
/// Cursor-key parameter that precedes a modifier in a modified CSI sequence.
const CSI_KEY_PARAM: u8 = b'1';
/// Separator between CSI parameters.
const PARAM_SEPARATOR: u8 = b';';

/// Encodes a key with the legacy terminal escape sequences, or `None` when the
/// key has no meaningful legacy encoding.
pub(super) fn encode(key: KeyEvent) -> Option<Vec<u8>> {
    let modifiers = key.modifiers;
    match key.code {
        KeyCode::Char(c) => Some(encode_char(c, modifiers)),
        KeyCode::Enter => Some(vec![CARRIAGE_RETURN]),
        KeyCode::Backspace => Some(vec![BACKSPACE_BYTE]),
        KeyCode::Tab => Some(vec![TAB_BYTE]),
        KeyCode::BackTab => Some(csi_final(b'Z', KeyModifiers::NONE)),
        KeyCode::Esc => Some(vec![ESC]),
        KeyCode::Left => Some(csi_final(b'D', modifiers)),
        KeyCode::Right => Some(csi_final(b'C', modifiers)),
        KeyCode::Up => Some(csi_final(b'A', modifiers)),
        KeyCode::Down => Some(csi_final(b'B', modifiers)),
        KeyCode::Home => Some(csi_final(b'H', modifiers)),
        KeyCode::End => Some(csi_final(b'F', modifiers)),
        KeyCode::PageUp => Some(csi_tilde(5, modifiers)),
        KeyCode::PageDown => Some(csi_tilde(6, modifiers)),
        KeyCode::Insert => Some(csi_tilde(2, modifiers)),
        KeyCode::Delete => Some(csi_tilde(3, modifiers)),
        KeyCode::F(n) => encode_function_key(n, modifiers),
        _ => None,
    }
}

/// Encodes a character key, folding Control to a control byte and prefixing an
/// escape for Alt. Under escape-code disambiguation crossterm reports the base
/// letter plus a Shift modifier (Shift+Alt+A arrives as `Char('a')` + `SHIFT`),
/// so the shifted glyph is resolved first, otherwise the Shift would be lost.
fn encode_char(c: char, modifiers: KeyModifiers) -> Vec<u8> {
    let c = if modifiers.contains(KeyModifiers::SHIFT) {
        shifted_char(c).unwrap_or(c)
    } else {
        c
    };
    let mut bytes = if modifiers.contains(KeyModifiers::CONTROL) {
        control_byte(c).map_or_else(|| c.to_string().into_bytes(), |byte| vec![byte])
    } else {
        c.to_string().into_bytes()
    };
    if modifiers.contains(KeyModifiers::ALT) && !bytes.is_empty() {
        bytes.insert(0, ESC);
    }
    bytes
}

/// The control byte for `Ctrl` + `c` across the ASCII control range, or `None`
/// when the character has no control mapping.
fn control_byte(c: char) -> Option<u8> {
    match c {
        ' ' | '@' => Some(NUL),
        '?' => Some(CTRL_QUESTION),
        '['..='_' | 'a'..='z' => Some(c as u8 & CONTROL_MASK),
        'A'..='Z' => Some(c as u8 & CONTROL_MASK),
        _ => None,
    }
}

/// Encodes a function key `F(n)` with its modifiers: SS3 for unmodified F1-F4,
/// the modified CSI form when modifiers are held, and CSI-tilde for F5-F12.
/// Returns `None` for an unsupported key number.
fn encode_function_key(n: u8, modifiers: KeyModifiers) -> Option<Vec<u8>> {
    if (FIRST_FUNCTION_KEY..FIRST_FUNCTION_KEY + SS3_FUNCTION_KEYS).contains(&n) {
        let final_byte = SS3_F1 + (n - FIRST_FUNCTION_KEY);
        if xterm_modifier_code(modifiers) > NO_MODIFIER {
            return Some(csi_final(final_byte, modifiers));
        }
        let mut bytes = SS3.to_vec();
        bytes.push(final_byte);
        return Some(bytes);
    }
    let index = n.checked_sub(FIRST_FUNCTION_KEY + SS3_FUNCTION_KEYS)?;
    let param = *CSI_FUNCTION_PARAMS.get(usize::from(index))?;
    Some(csi_tilde(param, modifiers))
}

/// Builds a CSI sequence ending in `final_byte`, inserting a `1;<modifier>`
/// parameter when modifiers are held (e.g. `ESC [ 1 ; 5 A` for Ctrl-Up).
fn csi_final(final_byte: u8, modifiers: KeyModifiers) -> Vec<u8> {
    let code = xterm_modifier_code(modifiers);
    let mut bytes = CSI.to_vec();
    if code > NO_MODIFIER {
        bytes.push(CSI_KEY_PARAM);
        bytes.push(PARAM_SEPARATOR);
        bytes.extend_from_slice(code.to_string().as_bytes());
    }
    bytes.push(final_byte);
    bytes
}

/// Builds a tilde CSI sequence `ESC [ number ~`, inserting a `;<modifier>`
/// parameter when modifiers are held (e.g. `ESC [ 5 ; 5 ~` for Ctrl-PageUp).
fn csi_tilde(number: u8, modifiers: KeyModifiers) -> Vec<u8> {
    let code = xterm_modifier_code(modifiers);
    let mut bytes = CSI.to_vec();
    bytes.extend_from_slice(number.to_string().as_bytes());
    if code > NO_MODIFIER {
        bytes.push(PARAM_SEPARATOR);
        bytes.extend_from_slice(code.to_string().as_bytes());
    }
    bytes.push(TILDE);
    bytes
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyEvent;

    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn plain_char_is_utf8() {
        assert_eq!(
            encode(key(KeyCode::Char('a'), KeyModifiers::NONE)),
            Some(b"a".to_vec())
        );
    }

    #[test]
    fn ctrl_letter_is_a_control_byte() {
        assert_eq!(
            encode(key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(vec![0x03])
        );
    }

    #[test]
    fn ctrl_non_letters_map_to_control_bytes() {
        assert_eq!(
            encode(key(KeyCode::Char(' '), KeyModifiers::CONTROL)),
            Some(vec![0x00])
        );
        assert_eq!(
            encode(key(KeyCode::Char('['), KeyModifiers::CONTROL)),
            Some(vec![0x1B])
        );
        assert_eq!(
            encode(key(KeyCode::Char('_'), KeyModifiers::CONTROL)),
            Some(vec![0x1F])
        );
        assert_eq!(
            encode(key(KeyCode::Char('?'), KeyModifiers::CONTROL)),
            Some(vec![0x7F])
        );
    }

    #[test]
    fn enter_is_carriage_return() {
        assert_eq!(
            encode(key(KeyCode::Enter, KeyModifiers::NONE)),
            Some(vec![b'\r'])
        );
    }

    #[test]
    fn up_arrow_is_a_csi_sequence() {
        assert_eq!(
            encode(key(KeyCode::Up, KeyModifiers::NONE)),
            Some(b"\x1b[A".to_vec())
        );
    }

    #[test]
    fn ctrl_up_carries_a_modifier_parameter() {
        assert_eq!(
            encode(key(KeyCode::Up, KeyModifiers::CONTROL)),
            Some(b"\x1b[1;5A".to_vec())
        );
    }

    #[test]
    fn page_up_is_a_tilde_sequence() {
        assert_eq!(
            encode(key(KeyCode::PageUp, KeyModifiers::NONE)),
            Some(b"\x1b[5~".to_vec())
        );
        assert_eq!(
            encode(key(KeyCode::PageUp, KeyModifiers::CONTROL)),
            Some(b"\x1b[5;5~".to_vec())
        );
    }

    #[test]
    fn alt_char_is_escape_prefixed() {
        assert_eq!(
            encode(key(KeyCode::Char('b'), KeyModifiers::ALT)),
            Some(vec![0x1B, b'b'])
        );
    }

    #[test]
    fn shift_is_applied_to_the_base_letter_disambiguation_reports() {
        // With host disambiguation, crossterm delivers the base letter plus a
        // Shift modifier rather than the pre-shifted glyph.
        assert_eq!(
            encode(key(KeyCode::Char('a'), KeyModifiers::SHIFT)),
            Some(b"A".to_vec())
        );
        assert_eq!(
            encode(key(
                KeyCode::Char('a'),
                KeyModifiers::SHIFT | KeyModifiers::ALT
            )),
            Some(vec![0x1B, b'A'])
        );
    }

    #[test]
    fn ctrl_shift_letter_folds_like_ctrl_letter() {
        assert_eq!(
            encode(key(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            Some(vec![0x03])
        );
    }

    #[test]
    fn function_keys_use_ss3_and_csi_sequences() {
        assert_eq!(
            encode(key(KeyCode::F(1), KeyModifiers::NONE)),
            Some(b"\x1bOP".to_vec())
        );
        assert_eq!(
            encode(key(KeyCode::F(4), KeyModifiers::NONE)),
            Some(b"\x1bOS".to_vec())
        );
        assert_eq!(
            encode(key(KeyCode::F(5), KeyModifiers::NONE)),
            Some(b"\x1b[15~".to_vec())
        );
        assert_eq!(
            encode(key(KeyCode::F(12), KeyModifiers::NONE)),
            Some(b"\x1b[24~".to_vec())
        );
    }

    #[test]
    fn modified_function_keys_use_the_csi_form() {
        assert_eq!(
            encode(key(KeyCode::F(1), KeyModifiers::CONTROL)),
            Some(b"\x1b[1;5P".to_vec())
        );
        assert_eq!(
            encode(key(KeyCode::F(5), KeyModifiers::SHIFT)),
            Some(b"\x1b[15;2~".to_vec())
        );
    }

    #[test]
    fn unsupported_function_key_is_ignored() {
        assert_eq!(encode(key(KeyCode::F(13), KeyModifiers::NONE)), None);
    }
}
