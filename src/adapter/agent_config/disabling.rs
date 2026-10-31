/// How an agent turns a server off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Disabling {
    /// By a flag in the entry itself, `enabled = false`, as Codex and Zed
    /// read it.
    Flag,
    /// Outside the entry, in lists muster reads only to warn and never
    /// edits, as Claude Code keeps them: no entry reads as turned off.
    Elsewhere,
}
