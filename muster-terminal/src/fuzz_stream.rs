/// One in this many pieces is a raw random byte.
const RAW_BYTE_ODDS: u64 = 6;
/// The largest number a random sequence parameter takes, near the columns
/// the glyphs land in.
const MAX_PARAM: u64 = 12;
/// The fixed pieces a stream is drawn from: glyphs of every width, marks,
/// the controls and sequences a line is drawn with, and sequences and
/// characters cut short.
const PIECES: &[&[u8]] = &[
    b"a",
    b" ",
    "你".as_bytes(),
    "🙂".as_bytes(),
    "\u{301}".as_bytes(),
    "\u{202e}".as_bytes(),
    b"\t",
    b"\r",
    b"\x08",
    b"\n",
    b"\x1b7",
    b"\x1b8",
    b"\x1b[s",
    b"\x1b[u",
    b"\x1b[3;5H",
    b"\x1b[2A",
    b"\x1b[B",
    b"\x1b[?1049h",
    b"\x1b[?1049l",
    b"\x1bc",
    b"\x1b[31m",
    b"\x1b]0;t\x07",
    b"\x1b[",
    b"\xc2",
];
/// The final bytes of the sequences drawn with a random parameter: erases
/// and column moves.
const MOVES: &[char] = &['K', 'G', 'C', 'D', 'J'];

/// A deterministic random stream of the pieces a process draws its lines
/// with, for tests that no output breaks a renderer; a xorshift generator,
/// so a failing seed repeats.
pub struct FuzzStream(u64);

impl FuzzStream {
    /// A stream started from `seed`, which must not be zero.
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// A number below `bound`.
    pub fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    /// The next piece: a fixed piece, an erase or move with a random
    /// parameter, or a raw byte.
    pub fn piece(&mut self) -> Vec<u8> {
        if self.below(RAW_BYTE_ODDS) == 0 {
            return vec![self.next().to_le_bytes()[0]];
        }
        if self.below(2) == 0 {
            let action = MOVES[self.index(MOVES.len())];
            return format!("\x1b[{}{action}", self.below(MAX_PARAM)).into_bytes();
        }
        PIECES[self.index(PIECES.len())].to_vec()
    }

    /// The next raw number.
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// An index below `len`.
    fn index(&mut self, len: usize) -> usize {
        usize::try_from(self.below(len as u64)).unwrap_or_default()
    }
}
