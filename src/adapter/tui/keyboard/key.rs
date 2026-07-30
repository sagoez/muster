//! The domain model of a key, classified independently of the negotiated
//! protocol. [`classify`] maps a crossterm key code (plus its keypad state) to a
//! [`Key`], and the encoder decides the wire bytes from the `Key`, the held
//! modifiers, and the protocol. The same classification drives [`held_encoding`],
//! which decides whether a key is tracked as held and how its later events are
//! encoded, so the encoder and the held-key tracking in the runtime agree.
//!
//! The Kitty codepoint and functional-key tables follow the Kitty keyboard
//! protocol's functional-key definitions and match what crossterm decodes.

use crossterm::event::{KeyCode, KeyEvent, KeyEventState, KeyModifiers, MediaKeyCode};

use super::protocol::KeyboardProtocol;

/// Modifiers that make a key escape-coded rather than literal text. Shift is
/// excluded: it yields the shifted character, not a `CSI u` event.
pub(super) const ESCAPE_CODED_MODIFIERS: KeyModifiers = KeyModifiers::CONTROL
    .union(KeyModifiers::ALT)
    .union(KeyModifiers::SUPER)
    .union(KeyModifiers::HYPER)
    .union(KeyModifiers::META);

/// Kitty `CSI u` codepoint for Enter.
pub(super) const ENTER: u32 = 13;
/// Kitty `CSI u` codepoint for Tab (and, with Shift, BackTab).
pub(super) const TAB: u32 = 9;
/// Kitty `CSI u` codepoint for Backspace.
pub(super) const BACKSPACE: u32 = 127;
/// Kitty `CSI u` codepoint for Escape.
pub(super) const ESCAPE: u32 = 27;
/// Carriage return, Enter's legacy byte.
const LEGACY_ENTER: &[u8] = b"\r";
/// Horizontal tab, Tab's legacy byte.
const LEGACY_TAB: &[u8] = b"\t";
/// Delete, Backspace's legacy byte.
const LEGACY_BACKSPACE: &[u8] = b"\x7f";
/// `CSI Z`, BackTab's legacy sequence.
const LEGACY_BACK_TAB: &[u8] = b"\x1b[Z";
/// First F-key with a private-use codepoint (F13); F1-F12 use legacy forms.
const F13_CODEPOINT: u32 = 57376;
/// Lowest F-key number with a Kitty codepoint.
const FIRST_CODEPOINT_FUNCTION_KEY: u8 = 13;
/// Highest F-key number with a Kitty codepoint.
const LAST_CODEPOINT_FUNCTION_KEY: u8 = 35;
/// Highest F-key number with a legacy CSI/tilde form.
const LAST_LEGACY_FUNCTION_KEY: u8 = 12;
/// Lowest F-key number encoded with the tilde form (F5).
const FIRST_TILDE_FUNCTION_KEY: u8 = 5;
/// Tilde parameters for F5 through F12, in order.
const FUNCTION_TILDE_PARAMS: [u32; 8] = [15, 17, 18, 19, 20, 21, 23, 24];
/// Tilde parameter for F3, whose legacy `CSI R` form collides with the
/// cursor-position report and is therefore retired in favour of the tilde form.
const F3_TILDE_PARAM: u32 = 13;

/// Functional-key codepoint for Caps Lock.
const CAPS_LOCK_CODE: u32 = 57358;
/// Functional-key codepoint for Scroll Lock.
const SCROLL_LOCK_CODE: u32 = 57359;
/// Functional-key codepoint for Num Lock.
const NUM_LOCK_CODE: u32 = 57360;
/// Functional-key codepoint for Print Screen.
const PRINT_SCREEN_CODE: u32 = 57361;
/// Functional-key codepoint for Pause.
const PAUSE_CODE: u32 = 57362;
/// Functional-key codepoint for Menu.
const MENU_CODE: u32 = 57363;
/// Codepoint for the numeric-keypad Begin (center) key.
const KEYPAD_BEGIN_CODE: u32 = 57427;
/// First media-key codepoint (Play); the rest follow contiguously.
const MEDIA_CODE_BASE: u32 = 57428;

/// Codepoint of the first keypad digit (KP_0); KP_1-KP_9 follow it.
const KEYPAD_ZERO: u32 = 57399;
/// Codepoint of the keypad decimal point.
const KEYPAD_DECIMAL: u32 = 57409;
/// Codepoint of the keypad divide key.
const KEYPAD_DIVIDE: u32 = 57410;
/// Codepoint of the keypad multiply key.
const KEYPAD_MULTIPLY: u32 = 57411;
/// Codepoint of the keypad subtract key.
const KEYPAD_SUBTRACT: u32 = 57412;
/// Codepoint of the keypad add key.
const KEYPAD_ADD: u32 = 57413;
/// Codepoint of the keypad Enter key.
const KEYPAD_ENTER: u32 = 57414;
/// Codepoint of the keypad equals key.
const KEYPAD_EQUAL: u32 = 57415;
/// Codepoint of the keypad separator key.
const KEYPAD_SEPARATOR: u32 = 57416;
/// Codepoint of the keypad Left arrow.
const KEYPAD_LEFT: u32 = 57417;
/// Codepoint of the keypad Right arrow.
const KEYPAD_RIGHT: u32 = 57418;
/// Codepoint of the keypad Up arrow.
const KEYPAD_UP: u32 = 57419;
/// Codepoint of the keypad Down arrow.
const KEYPAD_DOWN: u32 = 57420;
/// Codepoint of the keypad Page Up.
const KEYPAD_PAGE_UP: u32 = 57421;
/// Codepoint of the keypad Page Down.
const KEYPAD_PAGE_DOWN: u32 = 57422;
/// Codepoint of the keypad Home.
const KEYPAD_HOME: u32 = 57423;
/// Codepoint of the keypad End.
const KEYPAD_END: u32 = 57424;
/// Codepoint of the keypad Insert.
const KEYPAD_INSERT: u32 = 57425;
/// Codepoint of the keypad Delete.
const KEYPAD_DELETE: u32 = 57426;

/// The legacy CSI shape of a functional key, gaining Kitty modifier and event
/// parameters when the child enables event types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionalForm {
    /// `CSI 1 ; mods[:event] <final>` (arrows, Home, End, F1, F2, F4).
    Final(u8),
    /// `CSI <number> ; mods[:event] ~` (Page keys, Insert, Delete, F3, F5-F12).
    Tilde(u32),
}

/// A key classified for encoding, independent of the negotiated protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Key {
    /// A printable character: literal text, or a `CSI u` event under a control
    /// or alt modifier when the child disambiguates.
    Text(char),
    /// Enter, Tab, Backspace, or BackTab: their legacy byte when unmodified, or a
    /// `CSI codepoint u` event when modified and disambiguated.
    Ambiguous {
        codepoint: u32,
        legacy: &'static [u8],
    },
    /// Escape: `\x1b`, or `CSI 27 u` when the child disambiguates (even unmodified).
    Escape,
    /// A key with a legacy CSI/tilde form (cursor keys, F1-F12) that always
    /// encodes as an escape sequence and gains modifier and event parameters.
    Functional(FunctionalForm),
    /// A key representable only by its Kitty codepoint (F13-F35, keypad, Caps
    /// Lock, media): needs disambiguation to be sent at all.
    Codepoint(u32),
    /// A key muster does not encode.
    Ignored,
}

/// How a held key's later events (repeat, release) are encoded: the escape-coded
/// form fixed at press time. Only the modifier and event parameters change per
/// event; the codepoint or functional form stays constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeldEncoding {
    /// A `CSI codepoint ; modifier : event u` key.
    CsiU(u32),
    /// A functional key's legacy CSI/tilde form with modifier and event.
    Functional(FunctionalForm),
}

/// The encoding to reuse for a held key's repeat and release events, or `None`
/// when the key is press-only for this child (plain text, an unmodified legacy
/// key, or a child that did not enable event types). The runtime tracks a held
/// key exactly when this is `Some`, so it and the encoder agree, and the stored
/// form lets a later event apply its own current modifiers rather than the
/// press's - a release after the modifier is lifted still matches its press.
pub fn held_encoding(key: KeyEvent, protocol: KeyboardProtocol) -> Option<HeldEncoding> {
    if !protocol.reports_events() {
        return None;
    }
    let disambiguates = protocol.disambiguates();
    let escape_modified = key.modifiers.intersects(ESCAPE_CODED_MODIFIERS);
    // Control/Alt chords, Escape, and the ambiguous keys are tracked only when the
    // child disambiguates. Without flag 1 they are sent as legacy bytes (a control
    // code, a bare ESC, `\r`), and an event type can only attach to a key already
    // in `CSI u` form - there is no field on a raw byte to carry a `:2`/`:3` repeat
    // or release. So under REPORT_EVENT_TYPES alone they are correctly press-only;
    // this is not a stuck-key bug, since a legacy byte is not a tracked press in the
    // child. Do not add event-only tracking for those three guarded arms. This holds
    // even if the host reports a `CSI codepoint:3u` release for such a key (some
    // terminals do): muster sent the child a literal/legacy press, so relaying a
    // `CSI u` release the child cannot pair with it would be worse than dropping it -
    // muster would have to emit the press as `CSI u` too, which needs report-all
    // (flag 8), a mode muster does not support. Functional and codepoint keys, by
    // contrast, are always escape codes with no legacy form, so flag 2 alone reports
    // their repeats and releases.
    match classify(key.code, key.state) {
        Key::Text(ch) if escape_modified && disambiguates => Some(HeldEncoding::CsiU(u32::from(
            canonical_char(ch, key.modifiers),
        ))),
        Key::Ambiguous { codepoint, .. } if !key.modifiers.is_empty() && disambiguates => {
            Some(HeldEncoding::CsiU(codepoint))
        },
        Key::Escape if disambiguates => Some(HeldEncoding::CsiU(ESCAPE)),
        Key::Functional(form) => Some(HeldEncoding::Functional(form)),
        Key::Codepoint(codepoint) => Some(HeldEncoding::CsiU(codepoint)),
        _ => None,
    }
}

/// The Kitty codepoint for a character key, lowercasing a shifted letter so the
/// shift lives in the modifier field rather than the codepoint.
pub(super) fn canonical_char(ch: char, modifiers: KeyModifiers) -> char {
    if modifiers.contains(KeyModifiers::SHIFT) && ch.is_ascii_uppercase() {
        ch.to_ascii_lowercase()
    } else {
        ch
    }
}

/// The uppercase or shifted-symbol form of an ASCII character, or `None` when the
/// character has no distinct shifted form. Under escape-code disambiguation
/// crossterm delivers the base letter plus a Shift modifier, so both the legacy
/// and literal-text encoders resolve the shifted glyph from here.
///
/// Only letters and already-shifted symbols are resolved. Digits and unshifted
/// punctuation (e.g. `Shift+4`) are deliberately left unchanged: their shifted
/// glyph is keyboard-layout dependent (`Shift+3` is `#` on US, `£` on UK, `§` on
/// German), and crossterm cannot report it without `REPORT_ALTERNATE_KEYS` - which
/// muster intentionally does not request, because enabling it makes crossterm
/// clear the Shift modifier and emit the shifted codepoint, breaking the
/// `Ctrl+Shift+letter` `CSI u` encoding for Kitty children (`Ctrl+Shift+A` would
/// become `65;5u` instead of `97;6u`). A legacy pane therefore receives the base
/// character for `Shift+digit`/`Shift+punctuation` chords. This is a known,
/// accepted limitation forced by the absence of layout information, not an
/// oversight - do not add a hardcoded shifted-symbol table (it would be wrong for
/// non-US layouts).
pub(super) fn shifted_char(ch: char) -> Option<char> {
    if ch.is_ascii_uppercase() {
        return Some(ch);
    }
    if ch.is_ascii_lowercase() {
        return Some(ch.to_ascii_uppercase());
    }
    is_shifted_ascii_punctuation(ch).then_some(ch)
}

/// Whether `ch` is an ASCII symbol produced with Shift held.
fn is_shifted_ascii_punctuation(ch: char) -> bool {
    matches!(
        ch,
        '!' | '@'
            | '#'
            | '$'
            | '%'
            | '^'
            | '&'
            | '*'
            | '('
            | ')'
            | '_'
            | '+'
            | '{'
            | '}'
            | '|'
            | ':'
            | '"'
            | '<'
            | '>'
            | '?'
            | '~'
    )
}

/// Classifies a key code (with its keypad state) into a [`Key`]. Independent of
/// modifiers and protocol: it describes what the key is, not how it is encoded.
pub(super) fn classify(code: KeyCode, state: KeyEventState) -> Key {
    if state.contains(KeyEventState::KEYPAD)
        && let Some(codepoint) = keypad_codepoint(code)
    {
        return Key::Codepoint(codepoint);
    }
    match code {
        KeyCode::Char(ch) => Key::Text(ch),
        KeyCode::Enter => Key::Ambiguous {
            codepoint: ENTER,
            legacy: LEGACY_ENTER,
        },
        KeyCode::Tab => Key::Ambiguous {
            codepoint: TAB,
            legacy: LEGACY_TAB,
        },
        KeyCode::Backspace => Key::Ambiguous {
            codepoint: BACKSPACE,
            legacy: LEGACY_BACKSPACE,
        },
        KeyCode::BackTab => Key::Ambiguous {
            codepoint: TAB,
            legacy: LEGACY_BACK_TAB,
        },
        KeyCode::Esc => Key::Escape,
        KeyCode::Up => Key::Functional(FunctionalForm::Final(b'A')),
        KeyCode::Down => Key::Functional(FunctionalForm::Final(b'B')),
        KeyCode::Right => Key::Functional(FunctionalForm::Final(b'C')),
        KeyCode::Left => Key::Functional(FunctionalForm::Final(b'D')),
        KeyCode::Home => Key::Functional(FunctionalForm::Final(b'H')),
        KeyCode::End => Key::Functional(FunctionalForm::Final(b'F')),
        KeyCode::PageUp => Key::Functional(FunctionalForm::Tilde(5)),
        KeyCode::PageDown => Key::Functional(FunctionalForm::Tilde(6)),
        KeyCode::Insert => Key::Functional(FunctionalForm::Tilde(2)),
        KeyCode::Delete => Key::Functional(FunctionalForm::Tilde(3)),
        KeyCode::F(1) => Key::Functional(FunctionalForm::Final(b'P')),
        KeyCode::F(2) => Key::Functional(FunctionalForm::Final(b'Q')),
        KeyCode::F(3) => Key::Functional(FunctionalForm::Tilde(F3_TILDE_PARAM)),
        KeyCode::F(4) => Key::Functional(FunctionalForm::Final(b'S')),
        KeyCode::F(n @ FIRST_TILDE_FUNCTION_KEY..=LAST_LEGACY_FUNCTION_KEY) => {
            let param = FUNCTION_TILDE_PARAMS[usize::from(n - FIRST_TILDE_FUNCTION_KEY)];
            Key::Functional(FunctionalForm::Tilde(param))
        },
        KeyCode::F(n @ FIRST_CODEPOINT_FUNCTION_KEY..=LAST_CODEPOINT_FUNCTION_KEY) => {
            Key::Codepoint(F13_CODEPOINT + u32::from(n - FIRST_CODEPOINT_FUNCTION_KEY))
        },
        KeyCode::CapsLock => Key::Codepoint(CAPS_LOCK_CODE),
        KeyCode::ScrollLock => Key::Codepoint(SCROLL_LOCK_CODE),
        KeyCode::NumLock => Key::Codepoint(NUM_LOCK_CODE),
        KeyCode::PrintScreen => Key::Codepoint(PRINT_SCREEN_CODE),
        KeyCode::Pause => Key::Codepoint(PAUSE_CODE),
        KeyCode::Menu => Key::Codepoint(MENU_CODE),
        KeyCode::KeypadBegin => Key::Codepoint(KEYPAD_BEGIN_CODE),
        KeyCode::Media(media) => Key::Codepoint(media_codepoint(media)),
        _ => Key::Ignored,
    }
}

/// The distinct keypad codepoint for a key crossterm reported with the keypad
/// state bit, or `None` when the key has no keypad variant.
fn keypad_codepoint(code: KeyCode) -> Option<u32> {
    Some(match code {
        KeyCode::Char(c @ '0'..='9') => KEYPAD_ZERO + u32::from(c) - u32::from('0'),
        KeyCode::Char('.') => KEYPAD_DECIMAL,
        KeyCode::Char('/') => KEYPAD_DIVIDE,
        KeyCode::Char('*') => KEYPAD_MULTIPLY,
        KeyCode::Char('-') => KEYPAD_SUBTRACT,
        KeyCode::Char('+') => KEYPAD_ADD,
        KeyCode::Char('=') => KEYPAD_EQUAL,
        KeyCode::Char(',') => KEYPAD_SEPARATOR,
        KeyCode::Enter => KEYPAD_ENTER,
        KeyCode::Left => KEYPAD_LEFT,
        KeyCode::Right => KEYPAD_RIGHT,
        KeyCode::Up => KEYPAD_UP,
        KeyCode::Down => KEYPAD_DOWN,
        KeyCode::PageUp => KEYPAD_PAGE_UP,
        KeyCode::PageDown => KEYPAD_PAGE_DOWN,
        KeyCode::Home => KEYPAD_HOME,
        KeyCode::End => KEYPAD_END,
        KeyCode::Insert => KEYPAD_INSERT,
        KeyCode::Delete => KEYPAD_DELETE,
        _ => return None,
    })
}

/// The Kitty codepoint for a media key, laid out contiguously from Play.
fn media_codepoint(media: MediaKeyCode) -> u32 {
    let offset = match media {
        MediaKeyCode::Play => 0,
        MediaKeyCode::Pause => 1,
        MediaKeyCode::PlayPause => 2,
        MediaKeyCode::Reverse => 3,
        MediaKeyCode::Stop => 4,
        MediaKeyCode::FastForward => 5,
        MediaKeyCode::Rewind => 6,
        MediaKeyCode::TrackNext => 7,
        MediaKeyCode::TrackPrevious => 8,
        MediaKeyCode::Record => 9,
        MediaKeyCode::LowerVolume => 10,
        MediaKeyCode::RaiseVolume => 11,
        MediaKeyCode::MuteVolume => 12,
    };
    MEDIA_CODE_BASE + offset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn characters_are_text() {
        assert_eq!(
            classify(KeyCode::Char('a'), KeyEventState::NONE),
            Key::Text('a')
        );
    }

    #[test]
    fn a_keypad_key_takes_its_keypad_codepoint() {
        assert_eq!(
            classify(KeyCode::Left, KeyEventState::KEYPAD),
            Key::Codepoint(KEYPAD_LEFT)
        );
        assert_eq!(
            classify(KeyCode::Left, KeyEventState::NONE),
            Key::Functional(FunctionalForm::Final(b'D'))
        );
    }

    #[test]
    fn back_tab_is_the_tab_codepoint_with_a_csi_z_legacy() {
        assert_eq!(
            classify(KeyCode::BackTab, KeyEventState::NONE),
            Key::Ambiguous {
                codepoint: TAB,
                legacy: LEGACY_BACK_TAB,
            }
        );
    }

    #[test]
    fn f3_uses_the_tilde_form() {
        assert_eq!(
            classify(KeyCode::F(3), KeyEventState::NONE),
            Key::Functional(FunctionalForm::Tilde(F3_TILDE_PARAM))
        );
    }

    #[test]
    fn f13_and_functional_keys_are_codepoints() {
        assert_eq!(
            classify(KeyCode::F(13), KeyEventState::NONE),
            Key::Codepoint(F13_CODEPOINT)
        );
        assert_eq!(
            classify(KeyCode::CapsLock, KeyEventState::NONE),
            Key::Codepoint(CAPS_LOCK_CODE)
        );
    }

    fn kitty() -> KeyboardProtocol {
        KeyboardProtocol::Kitty {
            flags: crate::adapter::tui::keyboard::protocol::SUPPORTED_FLAGS,
        }
    }

    #[test]
    fn text_is_held_only_under_an_escape_coding_modifier() {
        let held = |mods| held_encoding(KeyEvent::new(KeyCode::Char('a'), mods), kitty());
        assert!(held(KeyModifiers::NONE).is_none());
        assert!(held(KeyModifiers::SHIFT).is_none());
        assert_eq!(held(KeyModifiers::CONTROL), Some(HeldEncoding::CsiU(97)));
        // Super, Hyper, and Meta are escape-coding modifiers too.
        assert_eq!(held(KeyModifiers::SUPER), Some(HeldEncoding::CsiU(97)));
    }

    #[test]
    fn an_unmodified_ambiguous_key_is_press_only() {
        let held = |mods| held_encoding(KeyEvent::new(KeyCode::Enter, mods), kitty());
        assert!(held(KeyModifiers::NONE).is_none());
        assert_eq!(held(KeyModifiers::SHIFT), Some(HeldEncoding::CsiU(ENTER)));
    }

    #[test]
    fn a_child_without_event_types_holds_nothing() {
        let disambiguate_only = KeyboardProtocol::Kitty { flags: 1 };
        assert!(
            held_encoding(
                KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
                disambiguate_only,
            )
            .is_none()
        );
    }
}
