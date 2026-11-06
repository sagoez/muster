use strum::{EnumIter, EnumMessage};

/// What a row says about its fact: how it is glyphed in the plain layout and
/// which kind of line it becomes in the interactive one. The plain layout
/// shows glyphs only for labeled rows, preserving the pipeable line format.
/// A kind's message is its status glyph; unglyphed kinds have none.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, EnumMessage)]
pub enum RowKind {
    /// Healthy.
    #[strum(message = "●")]
    Ok,
    /// Advisory.
    #[strum(message = "!")]
    Hint,
    /// Broken.
    #[strum(message = "✗")]
    Fail,
    /// Unstyled content.
    Plain,
    /// De-emphasized content.
    Dim,
}

#[cfg(test)]
mod tests {
    use strum::{EnumMessage, IntoEnumIterator};

    use super::RowKind;

    /// Only the status kinds carry a glyph.
    #[test]
    fn only_the_status_kinds_have_a_glyph() {
        for kind in RowKind::iter() {
            assert_eq!(
                kind.get_message().is_some(),
                matches!(kind, RowKind::Ok | RowKind::Hint | RowKind::Fail),
                "{kind:?}"
            );
        }
    }
}
