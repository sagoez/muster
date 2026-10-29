use std::{mem, str};

use super::{held_sequence::HeldSequence, scan_result::ScanResult};
use crate::domain::value::DetachKey;

/// The escape byte that starts every sequence a key can send.
const ESC: u8 = 0x1B;
/// The second byte of a control sequence.
const CSI_INTRODUCER: u8 = b'[';
/// The lowest final byte of a control sequence.
const CSI_FINAL_MIN: u8 = 0x40;
/// The highest final byte of a control sequence.
const CSI_FINAL_MAX: u8 = 0x7E;
/// Separates a control sequence's parameters.
const FIELD_SEPARATOR: char = ';';
/// Separates the parts of one Kitty parameter.
const PART_SEPARATOR: char = ':';
/// The final byte of a Kitty key report.
const KITTY_FINAL: u8 = b'u';
/// The final byte of an xterm modifyOtherKeys key report.
const MODIFY_OTHER_KEYS_FINAL: u8 = b'~';
/// The first parameter of an xterm modifyOtherKeys key report.
const MODIFY_OTHER_KEYS: &str = "27";
/// The modifier field of a key reported with no modifiers.
const NO_MODIFIERS: &str = "1";
/// The Kitty event type of a key release.
const RELEASE_EVENT: &str = "3";
/// Modifier bits for Caps Lock and Num Lock, which a terminal may report
/// with any key and which do not change which key it is.
const LOCK_MODIFIERS: u32 = 64 | 128;
/// Longest escape sequence held while it could still be the detach key.
const MAX_PENDING: usize = 32;

/// Finds the detach key in what a person types into `muster attach`, passing
/// every other byte on untouched.
pub struct DetachScanner {
    /// The byte the key sends without keyboard enhancement.
    legacy: u8,
    /// The key code and held modifiers of the key's enhanced press.
    kitty: Option<(u32, u32)>,
    /// The start of an escape sequence the chunk ended in, held until the
    /// next chunk shows whether it is the detach key.
    pending: Vec<u8>,
}

impl DetachScanner {
    /// A scanner for `key`.
    pub fn new(key: &DetachKey) -> Self {
        let kitty = key.kitty_sequence();
        Self {
            legacy: key.legacy_byte(),
            kitty: kitty
                .strip_prefix(&[ESC, CSI_INTRODUCER])
                .and_then(<[u8]>::split_last)
                .and_then(|(&last, params)| key_press(params, last)),
            pending: Vec::new(),
        }
    }

    /// Scans one chunk of typed input. An escape sequence the chunk ends in
    /// that could still be the detach key is held for the next chunk.
    pub fn feed(&mut self, bytes: &[u8]) -> ScanResult {
        let mut forward = Vec::new();
        for &byte in bytes {
            if self.pending.is_empty() {
                if byte == self.legacy {
                    return Self::result(forward, true);
                }
                if byte == ESC {
                    self.pending.push(byte);
                } else {
                    forward.push(byte);
                }
                continue;
            }
            self.pending.push(byte);
            match self.classify() {
                HeldSequence::Partial => {},
                HeldSequence::Detach => {
                    self.pending.clear();
                    return Self::result(forward, true);
                },
                HeldSequence::Other => forward.append(&mut self.pending),
                HeldSequence::Interrupted => {
                    self.pending.pop();
                    forward.append(&mut self.pending);
                    if byte == self.legacy {
                        return Self::result(forward, true);
                    }
                    if byte == ESC {
                        self.pending.push(byte);
                    } else {
                        forward.push(byte);
                    }
                },
            }
        }
        Self::result(forward, false)
    }

    /// What the held bytes, starting with an escape, have turned out to be.
    fn classify(&self) -> HeldSequence {
        let Some((&last, body)) = self.pending.split_last() else {
            return HeldSequence::Other;
        };
        match body {
            [ESC] if last == CSI_INTRODUCER => HeldSequence::Partial,
            [ESC] => HeldSequence::Interrupted,
            [ESC, CSI_INTRODUCER, params @ ..] => {
                if is_parameter(last) {
                    if self.pending.len() > MAX_PENDING {
                        HeldSequence::Other
                    } else {
                        HeldSequence::Partial
                    }
                } else if !(CSI_FINAL_MIN..=CSI_FINAL_MAX).contains(&last) {
                    HeldSequence::Interrupted
                } else if self.kitty.is_some() && key_press(params, last) == self.kitty {
                    HeldSequence::Detach
                } else {
                    HeldSequence::Other
                }
            },
            _ => HeldSequence::Other,
        }
    }

    /// A scan result passing `forward` on.
    fn result(forward: Vec<u8>, detached: bool) -> ScanResult {
        ScanResult::builder()
            .forward(forward)
            .detached(detached)
            .build()
    }

    /// Whether the last chunk ended in an escape sequence held back for the
    /// next one.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Gives up the held escape sequence, once no more input came to finish
    /// it, so a lone Escape key still reaches the process.
    pub fn flush(&mut self) -> Vec<u8> {
        mem::take(&mut self.pending)
    }
}

/// Whether `byte` can sit among a key report's parameters.
fn is_parameter(byte: u8) -> bool {
    byte.is_ascii_digit() || matches!(char::from(byte), FIELD_SEPARATOR | PART_SEPARATOR)
}

/// The key code and held modifier bits of a key press reported by a control
/// sequence with `params` and `final_byte`, in Kitty's form or xterm's
/// modifyOtherKeys form, with lock modifiers dropped; none for a release or
/// any other sequence.
fn key_press(params: &[u8], final_byte: u8) -> Option<(u32, u32)> {
    let fields: Vec<&str> = str::from_utf8(params)
        .ok()?
        .split(FIELD_SEPARATOR)
        .collect();
    let (code, modifiers) = match final_byte {
        KITTY_FINAL => {
            let code = fields.first()?.split(PART_SEPARATOR).next()?;
            let mut modifier = fields
                .get(1)
                .copied()
                .unwrap_or_default()
                .split(PART_SEPARATOR);
            let modifiers = modifier
                .next()
                .filter(|modifiers| !modifiers.is_empty())
                .unwrap_or(NO_MODIFIERS);
            if modifier.next() == Some(RELEASE_EVENT) {
                return None;
            }
            (code, modifiers)
        },
        MODIFY_OTHER_KEYS_FINAL => match fields.as_slice() {
            [MODIFY_OTHER_KEYS, modifiers, code] => (*code, *modifiers),
            _ => return None,
        },
        _ => return None,
    };
    let held = modifiers.parse::<u32>().ok()?.checked_sub(1)? & !LOCK_MODIFIERS;
    Some((code.parse().ok()?, held))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scanner(key: &str) -> DetachScanner {
        DetachScanner::new(&DetachKey::try_new(key).unwrap())
    }

    fn scanned(forward: &[u8], detached: bool) -> ScanResult {
        ScanResult::builder()
            .forward(forward.to_vec())
            .detached(detached)
            .build()
    }

    /// The legacy byte and every Kitty press of the key detach, whichever
    /// chunk boundary splits them; bytes before the key are passed on and
    /// bytes after it are not.
    #[test]
    fn detach_scanner_matches_legacy_and_kitty_forms_across_chunks() {
        let presses: [&[u8]; 7] = [
            b"\x1c",
            b"\x1b[92;5u",
            b"\x1b[92;5:1u",
            b"\x1b[92;5:2u",
            b"\x1b[92;69u",
            b"\x1b[92:124;5u",
            b"\x1b[27;5;92~",
        ];
        for press in presses {
            let typed = [b"ab".as_slice(), press, b"cd"].concat();
            for split in 0..=typed.len() {
                let mut scanner = scanner("ctrl-\\");
                let first = scanner.feed(&typed[..split]);
                let mut forward = first.forward().clone();
                let detached = first.detached() || {
                    let second = scanner.feed(&typed[split..]);
                    forward.extend(second.forward());
                    second.detached()
                };
                assert!(detached, "{press:?} split at {split} detaches");
                assert_eq!(forward, b"ab", "{press:?} split at {split}");
            }
        }

        let mut bracket = scanner("ctrl-]");
        assert_eq!(bracket.feed(b"x\x1d"), scanned(b"x", true));
        let mut bracket = scanner("ctrl-]");
        assert_eq!(bracket.feed(b"\x1b[93;5u"), scanned(b"", true));
    }

    /// Everything that is not the detach key, including other Control keys,
    /// the key's release, look-alike sequences and a lone Escape, passes on
    /// byte for byte.
    #[test]
    fn detach_scanner_forwards_everything_else_untouched() {
        let others: [&[u8]; 12] = [
            b"plain text \xe2\x9c\x93",
            b"\x03",
            b"\x1b[A",
            b"\x1b[1;5A",
            b"\x1b[97;5u",
            b"\x1b[92;5:3u",
            b"\x1b[92u",
            b"\x1b[92;3u",
            b"\x1b[<0;3;4M",
            b"\x1b[200~pasted\x1b[201~",
            b"\x1bOP",
            b"\x1b[27;5;97~",
        ];
        for other in others {
            let mut scanner = scanner("ctrl-\\");
            let mut forward = Vec::new();
            for byte in other {
                let result = scanner.feed(&[*byte]);
                assert!(!result.detached(), "{other:?} does not detach");
                forward.extend(result.forward());
            }
            forward.extend(scanner.flush());
            assert_eq!(forward, other, "{other:?} passes on whole");
        }

        let mut scanner = scanner("ctrl-\\");
        assert_eq!(scanner.feed(b"\x1b"), scanned(b"", false));
        assert!(scanner.has_pending(), "a lone Escape is held for its chunk");
        assert_eq!(scanner.flush(), b"\x1b");
        assert!(!scanner.has_pending());

        assert_eq!(scanner.feed(b"\x1b\x1b[92;5u"), scanned(b"\x1b", true));
        let mut interrupted = DetachScanner::new(&DetachKey::try_new("ctrl-\\").unwrap());
        assert_eq!(interrupted.feed(b"\x1b[\x1c"), scanned(b"\x1b[", true));
    }
}
