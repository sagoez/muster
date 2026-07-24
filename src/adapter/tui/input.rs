use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use vt100::{MouseProtocolEncoding, MouseProtocolMode};

/// Escape byte that introduces control sequences.
const ESC: u8 = 0x1B;
/// Control Sequence Introducer bytes (`ESC [`).
const CSI: &[u8] = b"\x1b[";
/// SS3 introducer bytes (`ESC O`), used by the unmodified F1-F4 sequences.
const SS3: &[u8] = b"\x1bO";
/// Carriage return sent for the Enter key.
const CARRIAGE_RETURN: u8 = b'\r';
/// Delete byte sent for Backspace.
const BACKSPACE_BYTE: u8 = 0x7F;
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
/// Modifier code that means "no modifiers".
const NO_MODIFIER: u8 = 1;
/// Base xterm button code for a left mouse button press.
const LEFT_BUTTON_CODE: u8 = 0;
/// Base xterm button code for a middle mouse button press.
const MIDDLE_BUTTON_CODE: u8 = 1;
/// Base xterm button code for a right mouse button press.
const RIGHT_BUTTON_CODE: u8 = 2;
/// xterm button code for a button release in legacy encodings.
const RELEASE_BUTTON_CODE: u8 = 3;
/// xterm button code bit that marks a mouse motion report.
const MOTION_BIT: u8 = 32;
/// xterm button code bit for an unpressed mouse motion report.
const NO_BUTTON_MOTION_CODE: u8 = RELEASE_BUTTON_CODE | MOTION_BIT;
/// xterm button code for wheel-up.
const SCROLL_UP_CODE: u8 = 64;
/// xterm button code for wheel-down.
const SCROLL_DOWN_CODE: u8 = 65;
/// xterm button code for wheel-left.
const SCROLL_LEFT_CODE: u8 = 66;
/// xterm button code for wheel-right.
const SCROLL_RIGHT_CODE: u8 = 67;
/// xterm button-code modifier bit for Shift.
const SHIFT_MOUSE_MODIFIER: u8 = 4;
/// xterm button-code modifier bit for Alt.
const ALT_MOUSE_MODIFIER: u8 = 8;
/// xterm button-code modifier bit for Control.
const CONTROL_MOUSE_MODIFIER: u8 = 16;
/// Default mouse-protocol offset for each encoded byte.
const LEGACY_MOUSE_OFFSET: u16 = 32;
/// The largest coordinate representable by the legacy one-byte protocol.
const MAX_LEGACY_MOUSE_COORDINATE: u16 = 223;
/// SGR mouse report prefix after CSI.
const SGR_MOUSE_PREFIX: u8 = b'<';
/// SGR mouse report terminator for a press or motion.
const SGR_MOUSE_PRESS: u8 = b'M';
/// SGR mouse report terminator for a release.
const SGR_MOUSE_RELEASE: u8 = b'm';
/// Cursor-up sequence in normal cursor-key mode (`CSI A`).
const CURSOR_UP_CSI: &[u8] = b"\x1b[A";
/// Cursor-down sequence in normal cursor-key mode (`CSI B`).
const CURSOR_DOWN_CSI: &[u8] = b"\x1b[B";
/// Cursor-up sequence in application cursor-key mode (`SS3 A`).
const CURSOR_UP_SS3: &[u8] = b"\x1bOA";
/// Cursor-down sequence in application cursor-key mode (`SS3 B`).
const CURSOR_DOWN_SS3: &[u8] = b"\x1bOB";

/// The cursor-key sequence sent to an alternate-screen child per wheel notch
/// (xterm alternate scroll), honoring DECCKM application cursor mode.
pub fn wheel_arrow(up: bool, application_cursor: bool) -> &'static [u8] {
    match (up, application_cursor) {
        (true, false) => CURSOR_UP_CSI,
        (false, false) => CURSOR_DOWN_CSI,
        (true, true) => CURSOR_UP_SS3,
        (false, true) => CURSOR_DOWN_SS3,
    }
}

/// Encodes a key event into the byte sequence a PTY expects, or `None` when the
/// key has no meaningful terminal encoding.
pub fn encode_key(key: KeyEvent) -> Option<Vec<u8>> {
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

/// Encodes a pointer event for a child that opted into xterm mouse reporting.
/// `column` and `row` are zero-based terminal coordinates.
pub fn encode_mouse(
    mouse: MouseEvent,
    column: u16,
    row: u16,
    mode: MouseProtocolMode,
    encoding: MouseProtocolEncoding,
) -> Option<Vec<u8>> {
    let (button, release) = mouse_button_code(mouse.kind, mode)?;
    let button = release_code(button, release, encoding)
        .saturating_add(mouse_modifier_bits(mouse.modifiers));
    let column = column.saturating_add(1);
    let row = row.saturating_add(1);
    match encoding {
        MouseProtocolEncoding::Sgr => Some(sgr_mouse_report(button, column, row, release)),
        MouseProtocolEncoding::Default => legacy_mouse_report(button, column, row),
        MouseProtocolEncoding::Utf8 => utf8_mouse_report(button, column, row),
    }
}

/// Maps a crossterm event to its xterm button code when the active mode reports it.
fn mouse_button_code(kind: MouseEventKind, mode: MouseProtocolMode) -> Option<(u8, bool)> {
    match kind {
        MouseEventKind::Down(button) if mode != MouseProtocolMode::None => {
            Some((button_code(button)?, false))
        },
        MouseEventKind::Up(button) if reports_release(mode) => Some((button_code(button)?, true)),
        MouseEventKind::Drag(button) if reports_motion(mode) => {
            Some((button_code(button)?.saturating_add(MOTION_BIT), false))
        },
        MouseEventKind::Moved if mode == MouseProtocolMode::AnyMotion => {
            Some((NO_BUTTON_MOTION_CODE, false))
        },
        MouseEventKind::ScrollUp if mode != MouseProtocolMode::None => {
            Some((SCROLL_UP_CODE, false))
        },
        MouseEventKind::ScrollDown if mode != MouseProtocolMode::None => {
            Some((SCROLL_DOWN_CODE, false))
        },
        MouseEventKind::ScrollLeft if mode != MouseProtocolMode::None => {
            Some((SCROLL_LEFT_CODE, false))
        },
        MouseEventKind::ScrollRight if mode != MouseProtocolMode::None => {
            Some((SCROLL_RIGHT_CODE, false))
        },
        _ => None,
    }
}

/// Uses SGR's original-button release code and legacy protocols' shared release code.
fn release_code(button: u8, release: bool, encoding: MouseProtocolEncoding) -> u8 {
    if release && encoding != MouseProtocolEncoding::Sgr {
        RELEASE_BUTTON_CODE
    } else {
        button
    }
}

/// Returns the xterm button code for a physical mouse button.
fn button_code(button: MouseButton) -> Option<u8> {
    match button {
        MouseButton::Left => Some(LEFT_BUTTON_CODE),
        MouseButton::Middle => Some(MIDDLE_BUTTON_CODE),
        MouseButton::Right => Some(RIGHT_BUTTON_CODE),
    }
}

/// Returns whether this xterm mode receives button releases.
fn reports_release(mode: MouseProtocolMode) -> bool {
    matches!(
        mode,
        MouseProtocolMode::PressRelease
            | MouseProtocolMode::ButtonMotion
            | MouseProtocolMode::AnyMotion
    )
}

/// Returns whether this xterm mode receives mouse motion while a button is held.
fn reports_motion(mode: MouseProtocolMode) -> bool {
    matches!(
        mode,
        MouseProtocolMode::ButtonMotion | MouseProtocolMode::AnyMotion
    )
}

/// Converts crossterm modifiers into xterm mouse button-code bits.
fn mouse_modifier_bits(modifiers: KeyModifiers) -> u8 {
    let mut bits = 0;
    if modifiers.contains(KeyModifiers::SHIFT) {
        bits += SHIFT_MOUSE_MODIFIER;
    }
    if modifiers.contains(KeyModifiers::ALT) {
        bits += ALT_MOUSE_MODIFIER;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        bits += CONTROL_MOUSE_MODIFIER;
    }
    bits
}

/// Builds an SGR mouse report, which has no coordinate-size limitation.
fn sgr_mouse_report(button: u8, column: u16, row: u16, release: bool) -> Vec<u8> {
    let terminator = if release {
        SGR_MOUSE_RELEASE
    } else {
        SGR_MOUSE_PRESS
    };
    let mut bytes = CSI.to_vec();
    bytes.push(SGR_MOUSE_PREFIX);
    bytes.extend(format!("{button};{column};{row}").bytes());
    bytes.push(terminator);
    bytes
}

/// Builds a legacy xterm mouse report, returning `None` outside its byte range.
fn legacy_mouse_report(button: u8, column: u16, row: u16) -> Option<Vec<u8>> {
    if column > MAX_LEGACY_MOUSE_COORDINATE || row > MAX_LEGACY_MOUSE_COORDINATE {
        return None;
    }
    Some(vec![
        ESC,
        b'[',
        b'M',
        button.saturating_add(LEGACY_MOUSE_OFFSET as u8),
        (column + LEGACY_MOUSE_OFFSET) as u8,
        (row + LEGACY_MOUSE_OFFSET) as u8,
    ])
}

/// Builds the UTF-8 extension of the legacy xterm mouse report.
fn utf8_mouse_report(button: u8, column: u16, row: u16) -> Option<Vec<u8>> {
    let mut bytes = vec![ESC, b'[', b'M'];
    for value in [
        u16::from(button) + LEGACY_MOUSE_OFFSET,
        column + LEGACY_MOUSE_OFFSET,
        row + LEGACY_MOUSE_OFFSET,
    ] {
        bytes.extend(char::from_u32(u32::from(value))?.to_string().bytes());
    }
    Some(bytes)
}

/// Encodes a character key, folding Control to a control byte and prefixing an
/// escape for Alt.
fn encode_char(c: char, modifiers: KeyModifiers) -> Vec<u8> {
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
        if modifier_code(modifiers) > NO_MODIFIER {
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
    let code = modifier_code(modifiers);
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
    let code = modifier_code(modifiers);
    let mut bytes = CSI.to_vec();
    bytes.extend_from_slice(number.to_string().as_bytes());
    if code > NO_MODIFIER {
        bytes.push(PARAM_SEPARATOR);
        bytes.extend_from_slice(code.to_string().as_bytes());
    }
    bytes.push(TILDE);
    bytes
}

/// The xterm modifier code: `1 + shift(1) + alt(2) + ctrl(4)`.
fn modifier_code(modifiers: KeyModifiers) -> u8 {
    let mut code = NO_MODIFIER;
    if modifiers.contains(KeyModifiers::SHIFT) {
        code += 1;
    }
    if modifiers.contains(KeyModifiers::ALT) {
        code += 2;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        code += 4;
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn plain_char_is_utf8() {
        assert_eq!(
            encode_key(key(KeyCode::Char('a'), KeyModifiers::NONE)),
            Some(b"a".to_vec())
        );
    }

    #[test]
    fn ctrl_letter_is_a_control_byte() {
        assert_eq!(
            encode_key(key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(vec![0x03])
        );
    }

    #[test]
    fn ctrl_non_letters_map_to_control_bytes() {
        assert_eq!(
            encode_key(key(KeyCode::Char(' '), KeyModifiers::CONTROL)),
            Some(vec![0x00])
        );
        assert_eq!(
            encode_key(key(KeyCode::Char('['), KeyModifiers::CONTROL)),
            Some(vec![0x1B])
        );
        assert_eq!(
            encode_key(key(KeyCode::Char('_'), KeyModifiers::CONTROL)),
            Some(vec![0x1F])
        );
        assert_eq!(
            encode_key(key(KeyCode::Char('?'), KeyModifiers::CONTROL)),
            Some(vec![0x7F])
        );
    }

    #[test]
    fn enter_is_carriage_return() {
        assert_eq!(
            encode_key(key(KeyCode::Enter, KeyModifiers::NONE)),
            Some(vec![b'\r'])
        );
    }

    #[test]
    fn up_arrow_is_a_csi_sequence() {
        assert_eq!(
            encode_key(key(KeyCode::Up, KeyModifiers::NONE)),
            Some(b"\x1b[A".to_vec())
        );
    }

    #[test]
    fn ctrl_up_carries_a_modifier_parameter() {
        assert_eq!(
            encode_key(key(KeyCode::Up, KeyModifiers::CONTROL)),
            Some(b"\x1b[1;5A".to_vec())
        );
    }

    #[test]
    fn page_up_is_a_tilde_sequence() {
        assert_eq!(
            encode_key(key(KeyCode::PageUp, KeyModifiers::NONE)),
            Some(b"\x1b[5~".to_vec())
        );
        assert_eq!(
            encode_key(key(KeyCode::PageUp, KeyModifiers::CONTROL)),
            Some(b"\x1b[5;5~".to_vec())
        );
    }

    #[test]
    fn alt_char_is_escape_prefixed() {
        assert_eq!(
            encode_key(key(KeyCode::Char('b'), KeyModifiers::ALT)),
            Some(vec![0x1B, b'b'])
        );
    }

    #[test]
    fn function_keys_use_ss3_and_csi_sequences() {
        assert_eq!(
            encode_key(key(KeyCode::F(1), KeyModifiers::NONE)),
            Some(b"\x1bOP".to_vec())
        );
        assert_eq!(
            encode_key(key(KeyCode::F(4), KeyModifiers::NONE)),
            Some(b"\x1bOS".to_vec())
        );
        assert_eq!(
            encode_key(key(KeyCode::F(5), KeyModifiers::NONE)),
            Some(b"\x1b[15~".to_vec())
        );
        assert_eq!(
            encode_key(key(KeyCode::F(12), KeyModifiers::NONE)),
            Some(b"\x1b[24~".to_vec())
        );
    }

    #[test]
    fn modified_function_keys_use_the_csi_form() {
        assert_eq!(
            encode_key(key(KeyCode::F(1), KeyModifiers::CONTROL)),
            Some(b"\x1b[1;5P".to_vec())
        );
        assert_eq!(
            encode_key(key(KeyCode::F(5), KeyModifiers::SHIFT)),
            Some(b"\x1b[15;2~".to_vec())
        );
    }

    #[test]
    fn unsupported_function_key_is_ignored() {
        assert_eq!(encode_key(key(KeyCode::F(13), KeyModifiers::NONE)), None);
    }

    /// SGR reports use one-based coordinates and preserve modifier bits.
    #[test]
    fn sgr_mouse_press_uses_terminal_coordinates() {
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::CONTROL,
        };

        assert_eq!(
            encode_mouse(
                mouse,
                4,
                2,
                MouseProtocolMode::ButtonMotion,
                MouseProtocolEncoding::Sgr,
            ),
            Some(b"\x1b[<16;5;3M".to_vec())
        );
    }

    /// Button-motion mode sends drag reports while press-release mode does not.
    #[test]
    fn mouse_motion_respects_the_child_requested_mode() {
        let mouse = MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };

        assert_eq!(
            encode_mouse(
                mouse,
                0,
                0,
                MouseProtocolMode::PressRelease,
                MouseProtocolEncoding::Sgr,
            ),
            None
        );
        assert_eq!(
            encode_mouse(
                mouse,
                0,
                0,
                MouseProtocolMode::ButtonMotion,
                MouseProtocolEncoding::Sgr,
            ),
            Some(b"\x1b[<32;1;1M".to_vec())
        );
    }

    /// SGR releases retain the released button so terminal applications can distinguish it.
    #[test]
    fn sgr_mouse_release_keeps_the_button_code() {
        let mouse = MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Right),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };

        assert_eq!(
            encode_mouse(
                mouse,
                1,
                3,
                MouseProtocolMode::PressRelease,
                MouseProtocolEncoding::Sgr,
            ),
            Some(b"\x1b[<2;2;4m".to_vec())
        );
    }

    /// Legacy mouse reports do not silently wrap coordinates they cannot encode.
    #[test]
    fn legacy_mouse_reports_reject_large_coordinates() {
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };

        assert_eq!(
            encode_mouse(
                mouse,
                MAX_LEGACY_MOUSE_COORDINATE,
                0,
                MouseProtocolMode::Press,
                MouseProtocolEncoding::Default,
            ),
            None
        );
    }
}
