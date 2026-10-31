/// How an agent turns a server off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Disabling {
    /// By a flag in the entry itself, `enabled = false`, as Codex and Zed
    /// read it.
    Flag,
    /// Outside the entry, where muster does not look, as Claude Code keeps
    /// it: no entry reads as turned off.
    Elsewhere,
}
