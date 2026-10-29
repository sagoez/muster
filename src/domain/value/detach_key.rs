use nutype::nutype;

/// Prefix naming the Control modifier, the only modifier a detach key takes.
const CTRL_PREFIX: &str = "ctrl-";
/// Bits a Control chord keeps of its key's ASCII code in the legacy encoding.
const LEGACY_CONTROL_MASK: u8 = 0x1F;
/// Kitty's modifier field for Control alone: one plus the Control bit (4).
const KITTY_CTRL_MODIFIER: u8 = 5;
/// Punctuation a detach chord may use besides letters: keys typed without
/// Shift whose Control chord has its own legacy byte, so both encodings name
/// the same key. `[` is left out, since its chord is Escape, which starts
/// every terminal escape sequence; `^`, `_` and `@` need Shift, which the
/// Kitty encoding would report.
const DETACH_PUNCTUATION: &[char] = &['\\', ']'];
/// Letters whose Control chord is a key terminals send on their own: Enter
/// (`m`), line feed (`j`), Tab (`i`), Backspace (`h`) and interrupt (`c`).
/// Detaching on one would end a session on an ordinary keystroke.
const RESERVED_KEYS: &[char] = &['m', 'j', 'i', 'h', 'c'];

/// The key of `chord` (`ctrl-<key>`), when it names one a detach chord may use.
fn chord_key(chord: &str) -> Option<char> {
    let mut chars = chord.strip_prefix(CTRL_PREFIX)?.chars();
    let key = chars.next()?;
    let usable = (key.is_ascii_lowercase() && !RESERVED_KEYS.contains(&key))
        || DETACH_PUNCTUATION.contains(&key);
    (chars.next().is_none() && usable).then_some(key)
}

/// Whether `chord` names a Control chord a detach key may use.
fn is_detach_chord(chord: &str) -> bool {
    chord_key(chord).is_some()
}

/// The single key that detaches `muster attach` from a process, written as a
/// Control chord such as `ctrl-\`.
#[nutype(
    sanitize(trim, lowercase),
    validate(predicate = is_detach_chord),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        AsRef,
        Display,
        FromStr,
        Serialize,
        Deserialize
    )
)]
pub struct DetachKey(String);

impl DetachKey {
    /// The byte a terminal without keyboard enhancement sends for this chord.
    pub fn legacy_byte(&self) -> u8 {
        self.key() as u8 & LEGACY_CONTROL_MASK
    }

    /// The Kitty keyboard protocol sequence (`CSI code ; modifiers u`) a
    /// terminal with keyboard enhancement sends for this chord.
    pub fn kitty_sequence(&self) -> Vec<u8> {
        format!("\x1b[{};{KITTY_CTRL_MODIFIER}u", u32::from(self.key())).into_bytes()
    }

    /// The chord's key, which validation guarantees is present.
    fn key(&self) -> char {
        chord_key(self.as_ref()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detach_key_parses_and_encodes_legacy_and_kitty_forms() {
        let key: DetachKey = serde_yaml_ng::from_str(r#""ctrl-\\""#).unwrap();
        assert_eq!(key.as_ref(), "ctrl-\\");
        assert_eq!(key.legacy_byte(), 0x1C);
        assert_eq!(key.kitty_sequence(), b"\x1b[92;5u");

        let letter = DetachKey::try_new(" Ctrl-A ").unwrap();
        assert_eq!(letter.as_ref(), "ctrl-a");
        assert_eq!(letter.legacy_byte(), 0x01);
        assert_eq!(letter.kitty_sequence(), b"\x1b[97;5u");

        assert_eq!(DetachKey::try_new("ctrl-]").unwrap().legacy_byte(), 0x1D);
    }

    #[test]
    fn a_detach_key_must_be_one_usable_control_chord() {
        for refused in [
            "", "a", "ctrl-", "ctrl-ab", "alt-a", "ctrl-[", "ctrl-1", "ctrl-^", "ctrl-_", "ctrl-@",
            "ctrl-m", "ctrl-j", "ctrl-i", "ctrl-h", "ctrl-c", "Ctrl-M",
        ] {
            assert!(
                DetachKey::try_new(refused).is_err(),
                "{refused:?} is not a detach key"
            );
        }
    }
}
