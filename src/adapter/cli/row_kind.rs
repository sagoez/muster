use super::report::{FAIL_GLYPH, HINT_GLYPH, OK_GLYPH};

/// What a row says about its fact: how it is glyphed in the plain layout and
/// which kind of line it becomes in the interactive one. The plain layout
/// shows glyphs only for labeled rows, preserving the pipeable line format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    /// Healthy.
    Ok,
    /// Advisory.
    Hint,
    /// Broken.
    Fail,
    /// Unstyled content.
    Plain,
    /// De-emphasized content.
    Dim,
}

impl RowKind {
    /// The status glyph for glyphed kinds.
    pub(super) fn glyph(self) -> Option<&'static str> {
        match self {
            RowKind::Ok => Some(OK_GLYPH),
            RowKind::Hint => Some(HINT_GLYPH),
            RowKind::Fail => Some(FAIL_GLYPH),
            RowKind::Plain | RowKind::Dim => None,
        }
    }
}
