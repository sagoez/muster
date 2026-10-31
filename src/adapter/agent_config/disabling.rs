/// How an agent turns a server off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Disabling {
    /// By a flag in the entry itself, `enabled = false`, as Codex and Zed
    /// read it.
    Flag,
    /// By lists of turned-off names kept outside the entry, as Claude Code
    /// keeps them; whether one of them names the entry.
    Listed(bool),
}
