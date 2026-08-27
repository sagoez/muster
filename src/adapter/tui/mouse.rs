//! Pointer and scroll-wheel encoding for children that opt into xterm mouse
//! reporting, across the legacy, UTF-8, and SGR protocol encodings.

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::adapter::tui::emulator::{MouseProtocolEncoding, MouseProtocolMode};

/// Escape byte that introduces control sequences.
const ESC: u8 = 0x1B;
/// Control Sequence Introducer bytes (`ESC [`).
const CSI: &[u8] = b"\x1b[";
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

#[cfg(test)]
mod tests {
    use super::*;

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
                MouseProtocolMode::PressRelease,
                MouseProtocolEncoding::Default,
            ),
            None
        );
    }
}
