/// How one character of shared text shows, as judged by what is around it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShownAs {
    /// As its rendering shows such a character anywhere.
    Usual,
    /// As its unicode escape: a mark with nothing to sit on, or one too many.
    Escaped,
    /// As itself, though it would usually be escaped: a joiner between the
    /// two characters it joins.
    Written,
}
