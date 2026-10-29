use std::mem;

use super::{
    held_sequence::HeldSequence, key_press::KeyPress, parsed::Parsed, scan_result::ScanResult,
};
use crate::{
    adapter::terminal::{
        bounded_parser::BoundedParser,
        escape::{CSI, ESC},
    },
    domain::value::DetachKey,
};

/// Longest escape sequence held while it could still be the detach key.
const MAX_PENDING: usize = 32;

/// Finds the detach key in what a person types into `muster attach`, passing
/// every other byte on untouched.
pub struct DetachScanner {
    /// The byte the key sends without keyboard enhancement.
    legacy: u8,
    /// The key code and held modifiers of the key's enhanced press.
    kitty: Option<KeyPress>,
    /// Parses the escape sequence being held.
    parser: BoundedParser,
    /// The start of an escape sequence, held until it shows whether it is
    /// the detach key.
    pending: Vec<u8>,
}

impl DetachScanner {
    /// A scanner for `key`.
    pub fn new(key: &DetachKey) -> Self {
        let mut parsed = Parsed::default();
        BoundedParser::new().advance(&mut parsed, &key.kitty_sequence());
        Self {
            legacy: key.legacy_byte(),
            kitty: parsed.press,
            parser: BoundedParser::new(),
            pending: Vec::new(),
        }
    }

    /// Scans one chunk of typed input. An escape sequence the chunk ends in
    /// that could still be the detach key is held for the next chunk.
    pub fn feed(&mut self, bytes: &[u8]) -> ScanResult {
        let mut forward = Vec::new();
        for &byte in bytes {
            if byte == self.legacy {
                forward.append(&mut self.flush());
                return Self::result(forward, true);
            }
            if byte == ESC {
                forward.append(&mut self.flush());
            } else if self.pending.is_empty() {
                forward.push(byte);
                continue;
            }
            self.pending.push(byte);
            match self.classify(byte) {
                HeldSequence::Partial => {},
                HeldSequence::Detach => {
                    self.flush();
                    return Self::result(forward, true);
                },
                HeldSequence::Other => forward.append(&mut self.flush()),
            }
        }
        Self::result(forward, false)
    }

    /// What the held bytes, starting with an escape and ending in `byte`,
    /// have turned out to be. Only a control sequence can be the detach key,
    /// so any other escape is passed on as soon as it starts.
    fn classify(&mut self, byte: u8) -> HeldSequence {
        let mut parsed = Parsed::default();
        self.parser.advance(&mut parsed, &[byte]);
        if parsed.completed {
            if self.kitty.is_some() && parsed.press == self.kitty {
                HeldSequence::Detach
            } else {
                HeldSequence::Other
            }
        } else if self.pending == [ESC]
            || (self.pending.starts_with(CSI.as_bytes()) && self.pending.len() <= MAX_PENDING)
        {
            HeldSequence::Partial
        } else {
            HeldSequence::Other
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
        self.parser = BoundedParser::new();
        mem::take(&mut self.pending)
    }
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
        let others: [&[u8]; 15] = [
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
            b"\x1b[>92;5u",
            b"\x1b[92;5$u",
            b"\x1b_x\x1b]y\x1bPz",
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

    /// An Alt chord that opens a string control on the parser, such as
    /// Alt+_ or Alt+], is passed on at once rather than held, so the keys
    /// after it are not swallowed into a string.
    #[test]
    fn an_escape_that_opens_a_string_is_passed_on_at_once() {
        for typed in [b"\x1b_".as_slice(), b"\x1b]", b"\x1bP", b"\x1bX"] {
            let mut scanner = scanner("ctrl-\\");
            assert_eq!(scanner.feed(typed), scanned(typed, false));
            assert!(!scanner.has_pending());
            assert_eq!(scanner.feed(b"ab\x1b[92;5u"), scanned(b"ab", true));
        }
    }
}
