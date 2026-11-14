use nutype::nutype;

/// A pane of a terminal workspace, as the workspace names it; opaque to
/// muster.
#[nutype(
    sanitize(trim),
    validate(not_empty),
    derive(Debug, Clone, PartialEq, Eq, AsRef, Display, Serialize)
)]
pub struct WorkspacePaneId(String);

#[cfg(test)]
mod tests {
    use super::*;

    /// An id is kept trimmed, and a blank one is refused.
    #[test]
    fn a_pane_id_is_trimmed_and_never_empty() {
        assert_eq!(
            WorkspacePaneId::try_new("  w1:p3\n").unwrap().as_ref(),
            "w1:p3"
        );
        assert!(WorkspacePaneId::try_new(" \t ").is_err());
        assert!(WorkspacePaneId::try_new("").is_err());
    }
}
