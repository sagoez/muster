/// What Codex's config holds for muster, against what an add wants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodexEntry {
    /// No muster server: `codex mcp add` registers one.
    Absent,
    /// A muster server already running this binary as muster's server.
    Current,
    /// A muster server to point at this binary: the config's new text,
    /// every other key of it and of the file kept as it was.
    Edited(String),
}
