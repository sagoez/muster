/// The read of one pane's log in flight.
pub(super) struct PendingRead {
    /// Its id, so a read that lands for a pane retired or replaced since is
    /// dropped.
    pub(super) id: u64,
    /// Whether it reads the log's end, which the waits held on a process
    /// this service never saw run take.
    pub(super) end: bool,
}
