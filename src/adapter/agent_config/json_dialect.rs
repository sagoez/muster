/// How strictly a JSON config file is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum JsonDialect {
    /// Plain JSON, as Claude Code reads it: a comment or trailing comma
    /// makes the file one muster leaves alone.
    Strict,
    /// JSON with comments and trailing commas, as Zed reads its settings.
    WithComments,
}
