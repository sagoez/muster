/// What an entry already under the name is to muster.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EntryMatch {
    /// The entry muster would write.
    Same,
    /// muster's own, launching a `muster` binary at another path, such as
    /// one an earlier install wrote.
    Older,
    /// Another server's.
    Foreign,
}
