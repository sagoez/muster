use crossbeam_channel::Sender;

/// One request to a [`NoticeMirror`](super::notice_mirror::NoticeMirror)'s
/// thread, applied in the order sent.
pub enum MirrorWrite {
    /// Appends a line, newline included.
    Line(Vec<u8>),
    /// Answers once every earlier line was written.
    Flush(Sender<()>),
}
