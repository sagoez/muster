//! The key-encoding entry point. A key is classified once into a [`Key`], then
//! encoded by a total match on that classification, the held modifiers, the
//! event kind, and the negotiated protocol. Legacy children (and Kitty keys that
//! fall back) use [`legacy`]; the Kitty path is expressed entirely as rules per
//! `Key` variant, with no guard chain.

use crossterm::event::{KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

use super::{
    key::{self, ESCAPE_CODED_MODIFIERS, FunctionalForm, HeldEncoding, Key, canonical_char},
    legacy,
    protocol::KeyboardProtocol,
};

/// Modifier code that means "no modifiers".
pub(super) const NO_MODIFIER: u8 = 1;
/// Kitty modifier bit for the Super modifier.
const SUPER: u16 = 8;
/// Kitty modifier bit for the Hyper modifier.
const HYPER: u16 = 16;
/// Kitty modifier bit for the Meta modifier.
const META: u16 = 32;
/// Kitty modifier bit set while Caps Lock is on.
const CAPS_LOCK: u16 = 64;
/// Kitty modifier bit set while Num Lock is on.
const NUM_LOCK: u16 = 128;
/// Kitty event-type code for a key press.
const EVENT_PRESS: u8 = 1;
/// Kitty event-type code for a key repeat.
const EVENT_REPEAT: u8 = 2;
/// Kitty event-type code for a key release.
const EVENT_RELEASE: u8 = 3;

/// The outcome of encoding a key under the Kitty protocol.
enum Encoded {
    /// Send these bytes to the child.
    Bytes(Vec<u8>),
    /// Handled, but produces nothing (a release the child did not ask for).
    Suppress,
    /// Not represented under Kitty; fall back to the legacy encoding.
    Defer,
}

/// Encodes a key event into the byte sequence a PTY child expects, or `None`
/// when the key produces nothing. A Kitty child gets `CSI u` and functional
/// events for the keys the protocol disambiguates; everything else, and every
/// legacy child, uses the traditional encoding.
pub fn encode_key(key: KeyEvent, protocol: KeyboardProtocol) -> Option<Vec<u8>> {
    if let KeyboardProtocol::Kitty { .. } = protocol {
        match encode_kitty(
            key::classify(key.code, key.state),
            key.modifiers,
            key.state,
            key.kind,
            protocol,
        ) {
            Encoded::Bytes(bytes) => return non_empty(bytes),
            Encoded::Suppress => return None,
            Encoded::Defer => {},
        }
    }
    // A release must never be re-sent as a legacy keystroke; only a Kitty child
    // that enabled event types (handled above) ever receives one.
    if key.kind == KeyEventKind::Release {
        return None;
    }
    legacy::encode(key)
}

/// Encodes a key for a Kitty child from its classification, modifiers, state,
/// and event kind.
///
/// The `disambiguates()` guards on text chords, modified Enter/Tab/Backspace, and
/// Escape are deliberate and spec-correct: only the disambiguate flag converts
/// these legacy-representable keys into `CSI u` form, so event-type reporting
/// alone cannot carry their press/repeat/release - there is no field on a raw
/// control byte or a bare ESC for a `:2`/`:3` sub-parameter. Event reporting adds
/// event types only to keys already sent as escape codes (the functional keys,
/// which carry no `disambiguates` guard). That a separate report-all-keys flag
/// exists confirms disambiguate and event reporting alone do not force these keys
/// into `CSI u`. Do not drop these guards to emit CSI-u under event types alone;
/// see the matching note in [`super::key`].
fn encode_kitty(
    key: Key,
    modifiers: KeyModifiers,
    state: KeyEventState,
    kind: KeyEventKind,
    protocol: KeyboardProtocol,
) -> Encoded {
    let event = event_code(kind, protocol.reports_events());
    // A release only ever produces bytes when the child enabled event types.
    if kind == KeyEventKind::Release && event.is_none() {
        return Encoded::Suppress;
    }
    let modified = modifiers.intersects(ESCAPE_CODED_MODIFIERS);

    match key {
        // Plain text is literal; a control/alt chord becomes a `CSI u` event when
        // disambiguated, and falls back to its legacy control byte otherwise.
        Key::Text(ch) if modified => {
            if protocol.disambiguates() {
                Encoded::Bytes(csi_u(
                    u32::from(canonical_char(ch, modifiers)),
                    modifiers,
                    state,
                    event,
                ))
            } else {
                Encoded::Defer
            }
        },
        Key::Text(ch) => match kind {
            KeyEventKind::Release => Encoded::Suppress,
            _ => Encoded::Bytes(text_literal(ch, modifiers)),
        },
        // Enter/Tab/Backspace/BackTab: a `CSI u` event when modified and
        // disambiguated, otherwise their legacy byte (and no release).
        Key::Ambiguous { codepoint, legacy } => {
            if !modifiers.is_empty() && protocol.disambiguates() {
                Encoded::Bytes(csi_u(codepoint, modifiers, state, event))
            } else if kind == KeyEventKind::Release {
                Encoded::Suppress
            } else {
                Encoded::Bytes(legacy.to_vec())
            }
        },
        // Escape becomes `CSI 27 u` under disambiguation even when unmodified.
        Key::Escape => {
            if protocol.disambiguates() {
                Encoded::Bytes(csi_u(key::ESCAPE, modifiers, state, event))
            } else if kind == KeyEventKind::Release {
                Encoded::Suppress
            } else {
                Encoded::Bytes(vec![0x1B])
            }
        },
        // Cursor and F1-F12 keys take their Kitty CSI/tilde form whenever the
        // child disambiguates (carrying the event suffix too under event types).
        // This is required for F3, whose retired legacy `R` form collides with the
        // cursor-position report; a purely legacy child gets the legacy form.
        Key::Functional(form) if protocol.disambiguates() || event.is_some() => Encoded::Bytes(
            encode_functional(form, kitty_modifier(modifiers, state), event),
        ),
        Key::Functional(_) => Encoded::Defer,
        // F13-F35, keypad, Caps Lock, media: Kitty functional keys with no legacy
        // form, so like the cursor/F1-F12 keys they take their `CSI u` codepoint
        // (with the event suffix) whenever the child disambiguates or reports
        // events - never dropped or collapsed under event types alone.
        Key::Codepoint(codepoint) => {
            if protocol.disambiguates() || event.is_some() {
                Encoded::Bytes(csi_u(codepoint, modifiers, state, event))
            } else if kind == KeyEventKind::Release {
                Encoded::Suppress
            } else {
                Encoded::Defer
            }
        },
        Key::Ignored => Encoded::Defer,
    }
}

/// Builds a `CSI codepoint ; modifier [: event] u` sequence.
fn csi_u(
    codepoint: u32,
    modifiers: KeyModifiers,
    state: KeyEventState,
    event: Option<u8>,
) -> Vec<u8> {
    let modifier = kitty_modifier(modifiers, state);
    match event {
        Some(event) => format!("\x1b[{codepoint};{modifier}:{event}u"),
        None => format!("\x1b[{codepoint};{modifier}u"),
    }
    .into_bytes()
}

/// Builds the Kitty functional-key form: the CSI/tilde sequence carrying the
/// `modifier` and optional `event` parameters, which are omitted entirely when
/// the key is unmodified with no event reported.
fn encode_functional(form: FunctionalForm, modifier: u16, event: Option<u8>) -> Vec<u8> {
    // Parameters appear when the key is modified or an event type is reported; an
    // unmodified key with no event omits them, matching the Kitty legacy
    // functional forms (`CSI A`, `CSI P`, `CSI 13~`) rather than a redundant
    // `1;1` modifier parameter.
    let params = if modifier != u16::from(NO_MODIFIER) || event.is_some() {
        match event {
            Some(event) => format!(";{modifier}:{event}"),
            None => format!(";{modifier}"),
        }
    } else {
        String::new()
    };
    match form {
        FunctionalForm::Final(final_byte) => {
            let lead = if params.is_empty() { "" } else { "1" };
            format!("\x1b[{lead}{params}{}", final_byte as char)
        },
        FunctionalForm::Tilde(number) => format!("\x1b[{number}{params}~"),
    }
    .into_bytes()
}

/// The literal UTF-8 bytes for a text key, applying the shifted form when Shift
/// is the only modifier.
fn text_literal(ch: char, modifiers: KeyModifiers) -> Vec<u8> {
    let ch = if modifiers == KeyModifiers::SHIFT {
        key::shifted_char(ch).unwrap_or(ch)
    } else {
        ch
    };
    ch.to_string().into_bytes()
}

/// Encodes a repeat or release of a held key. The escape-coded form was fixed
/// at press time (`event.encoding`); this applies the modifiers, state, and kind
/// of the current event, so a chord whose modifier was lifted still reports the
/// key's release with the modifier value in effect when it occurred.
pub fn encode_held(encoding: HeldEncoding, event: KeyEvent) -> Vec<u8> {
    let modifier = kitty_modifier(event.modifiers, event.state);
    let code = event_kind_code(event.kind);
    match encoding {
        HeldEncoding::CsiU(codepoint) => {
            format!("\x1b[{codepoint};{modifier}:{code}u").into_bytes()
        },
        HeldEncoding::Functional(form) => encode_functional(form, modifier, Some(code)),
    }
}

/// The Kitty event-type code for `kind`. A held key is only tracked once the
/// child enabled event types, so its later events always carry a suffix.
fn event_kind_code(kind: KeyEventKind) -> u8 {
    match kind {
        KeyEventKind::Press => EVENT_PRESS,
        KeyEventKind::Repeat => EVENT_REPEAT,
        KeyEventKind::Release => EVENT_RELEASE,
    }
}

/// The Kitty event-type code for `kind`, present only when the child enabled
/// event-type reporting.
fn event_code(kind: KeyEventKind, reports_events: bool) -> Option<u8> {
    reports_events.then_some(match kind {
        KeyEventKind::Press => EVENT_PRESS,
        KeyEventKind::Repeat => EVENT_REPEAT,
        KeyEventKind::Release => EVENT_RELEASE,
    })
}

/// The Kitty modifier code: xterm's `1 + shift + alt + ctrl` plus the Super,
/// Hyper, Meta, and Caps/Num Lock bits.
fn kitty_modifier(modifiers: KeyModifiers, state: KeyEventState) -> u16 {
    let mut code = u16::from(xterm_modifier_code(modifiers));
    if modifiers.contains(KeyModifiers::SUPER) {
        code += SUPER;
    }
    if modifiers.contains(KeyModifiers::HYPER) {
        code += HYPER;
    }
    if modifiers.contains(KeyModifiers::META) {
        code += META;
    }
    if state.contains(KeyEventState::CAPS_LOCK) {
        code += CAPS_LOCK;
    }
    if state.contains(KeyEventState::NUM_LOCK) {
        code += NUM_LOCK;
    }
    code
}

/// The xterm modifier code: `1 + shift(1) + alt(2) + ctrl(4)`, shared with the
/// legacy CSI builders.
pub(super) fn xterm_modifier_code(modifiers: KeyModifiers) -> u8 {
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

/// Wraps encoded bytes as `Some`, or `None` when empty.
fn non_empty(bytes: Vec<u8>) -> Option<Vec<u8>> {
    (!bytes.is_empty()).then_some(bytes)
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyCode;

    use super::*;
    use crate::adapter::tui::keyboard::protocol::{
        DISAMBIGUATE, REPORT_EVENT_TYPES, SUPPORTED_FLAGS,
    };

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn legacy(key: KeyEvent) -> Option<Vec<u8>> {
        encode_key(key, KeyboardProtocol::Legacy)
    }

    fn kitty(key: KeyEvent) -> Option<Vec<u8>> {
        encode_key(key, KeyboardProtocol::Kitty {
            flags: SUPPORTED_FLAGS,
        })
    }

    fn disambiguate_only(key: KeyEvent) -> Option<Vec<u8>> {
        encode_key(key, KeyboardProtocol::Kitty {
            flags: DISAMBIGUATE,
        })
    }

    fn event_only(key: KeyEvent) -> Option<Vec<u8>> {
        encode_key(key, KeyboardProtocol::Kitty {
            flags: REPORT_EVENT_TYPES,
        })
    }

    #[test]
    fn legacy_shift_enter_is_a_carriage_return() {
        assert_eq!(
            legacy(key(KeyCode::Enter, KeyModifiers::SHIFT)),
            Some(b"\r".to_vec())
        );
    }

    #[test]
    fn f3_uses_the_kitty_tilde_form_under_disambiguation() {
        // F3's legacy `R` final byte collides with the cursor-position report, so a
        // disambiguating child must receive the CSI 13~ form even without events.
        assert_eq!(
            disambiguate_only(key(KeyCode::F(3), KeyModifiers::NONE)),
            Some(b"\x1b[13~".to_vec())
        );
        assert_eq!(
            disambiguate_only(key(KeyCode::F(3), KeyModifiers::CONTROL)),
            Some(b"\x1b[13;5~".to_vec())
        );
    }

    #[test]
    fn functional_keys_take_kitty_forms_under_disambiguation() {
        // Unmodified cursor keys omit the modifier parameter; F1-F4 use the CSI
        // legacy-functional form rather than SS3; modifiers still attach.
        assert_eq!(
            disambiguate_only(key(KeyCode::Up, KeyModifiers::NONE)),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            disambiguate_only(key(KeyCode::F(1), KeyModifiers::NONE)),
            Some(b"\x1b[P".to_vec())
        );
        assert_eq!(
            disambiguate_only(key(KeyCode::Up, KeyModifiers::CONTROL)),
            Some(b"\x1b[1;5A".to_vec())
        );
    }

    #[test]
    fn a_legacy_child_still_gets_the_legacy_functional_forms() {
        assert_eq!(
            legacy(key(KeyCode::F(1), KeyModifiers::NONE)),
            Some(b"\x1bOP".to_vec())
        );
        assert_eq!(
            legacy(key(KeyCode::F(3), KeyModifiers::NONE)),
            Some(b"\x1bOR".to_vec())
        );
    }

    #[test]
    fn kitty_shift_enter_is_a_csi_u_event() {
        assert_eq!(
            kitty(key(KeyCode::Enter, KeyModifiers::SHIFT)),
            Some(b"\x1b[13;2:1u".to_vec())
        );
    }

    #[test]
    fn kitty_plain_enter_stays_a_carriage_return() {
        assert_eq!(
            kitty(key(KeyCode::Enter, KeyModifiers::NONE)),
            Some(b"\r".to_vec())
        );
    }

    #[test]
    fn kitty_plain_text_stays_literal() {
        assert_eq!(
            kitty(key(KeyCode::Char('a'), KeyModifiers::NONE)),
            Some(b"a".to_vec())
        );
    }

    #[test]
    fn kitty_ctrl_char_is_a_csi_u_event() {
        assert_eq!(
            kitty(key(KeyCode::Char('a'), KeyModifiers::CONTROL)),
            Some(b"\x1b[97;5:1u".to_vec())
        );
    }

    #[test]
    fn event_types_without_disambiguation_keep_ctrl_c_legacy() {
        // Only REPORT_EVENT_TYPES (bit 2), no disambiguation: Ctrl+C is ETX.
        let protocol = KeyboardProtocol::Kitty {
            flags: REPORT_EVENT_TYPES,
        };
        assert_eq!(
            encode_key(key(KeyCode::Char('c'), KeyModifiers::CONTROL), protocol),
            Some(vec![0x03])
        );
    }

    #[test]
    fn kitty_ctrl_backspace_is_a_csi_u_event() {
        assert_eq!(
            kitty(key(KeyCode::Backspace, KeyModifiers::CONTROL)),
            Some(b"\x1b[127;5:1u".to_vec())
        );
    }

    #[test]
    fn kitty_plain_escape_is_disambiguated() {
        assert_eq!(
            kitty(key(KeyCode::Esc, KeyModifiers::NONE)),
            Some(b"\x1b[27;1:1u".to_vec())
        );
    }

    #[test]
    fn kitty_shift_tab_is_the_tab_codepoint() {
        assert_eq!(
            kitty(key(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Some(b"\x1b[9;2:1u".to_vec())
        );
    }

    #[test]
    fn kitty_event_type_arrow_keeps_its_regular_csi_identity() {
        assert_eq!(
            kitty(key(KeyCode::Up, KeyModifiers::CONTROL)),
            Some(b"\x1b[1;5:1A".to_vec())
        );
    }

    #[test]
    fn kitty_f3_uses_the_tilde_form() {
        assert_eq!(
            kitty(key(KeyCode::F(3), KeyModifiers::NONE)),
            Some(b"\x1b[13;1:1~".to_vec())
        );
    }

    #[test]
    fn kitty_f13_uses_its_codepoint() {
        assert_eq!(
            kitty(key(KeyCode::F(13), KeyModifiers::NONE)),
            Some(b"\x1b[57376;1:1u".to_vec())
        );
    }

    #[test]
    fn functional_codepoints_carry_events_without_disambiguation() {
        // F13 has no legacy form; under event types it must still be a CSI u event
        // carrying the press type, never dropped.
        assert_eq!(
            event_only(key(KeyCode::F(13), KeyModifiers::NONE)),
            Some(b"\x1b[57376;1:1u".to_vec())
        );
        // Keypad keys keep their distinct codepoint and event type too.
        let keypad_left = KeyEvent::new_with_kind_and_state(
            KeyCode::Left,
            KeyModifiers::NONE,
            KeyEventKind::Press,
            KeyEventState::KEYPAD,
        );
        assert_eq!(event_only(keypad_left), Some(b"\x1b[57417;1:1u".to_vec()));
    }

    #[test]
    fn a_release_without_event_types_produces_nothing() {
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('a'),
            KeyModifiers::CONTROL,
            KeyEventKind::Release,
        );
        assert_eq!(
            encode_key(release, KeyboardProtocol::Kitty { flags: 1 }),
            None
        );
        assert_eq!(encode_key(release, KeyboardProtocol::Legacy), None);
    }

    #[test]
    fn a_ctrl_char_release_is_forwarded_under_event_types() {
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('z'),
            KeyModifiers::CONTROL,
            KeyEventKind::Release,
        );
        assert_eq!(kitty(release), Some(b"\x1b[122;5:3u".to_vec()));
    }

    #[test]
    fn caps_lock_state_sets_the_modifier_bit() {
        let mut event = key(KeyCode::Char('a'), KeyModifiers::CONTROL);
        event.state = KeyEventState::CAPS_LOCK;
        // Ctrl (4) + base (1) + Caps Lock (64) = 69.
        assert_eq!(kitty(event), Some(b"\x1b[97;69:1u".to_vec()));
    }
}
