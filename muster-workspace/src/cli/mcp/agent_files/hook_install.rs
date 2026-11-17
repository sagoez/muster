/// What putting muster's hook into a hooks file did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookInstall {
    /// The file held no muster hook; muster's hook was added.
    Added,
    /// The file held muster hooks running these binaries, gone since, and
    /// each now runs the binary asked for, the rest of its group kept.
    Repaired(Vec<String>),
    /// The file already held a muster hook whose binary is there, left as
    /// it is.
    Present,
}

impl HookInstall {
    /// Whether the file changed.
    pub fn changed(&self) -> bool {
        !matches!(self, Self::Present)
    }
}
