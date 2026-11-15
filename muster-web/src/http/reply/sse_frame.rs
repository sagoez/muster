/// The field telling a page how long to wait before reconnecting.
const RETRY_FIELD: &str = "retry: ";
/// The field that names a frame's event.
pub(crate) const EVENT_FIELD: &str = "event: ";
/// The field that carries a frame's id, which a reconnecting browser sends
/// back.
pub(crate) const ID_FIELD: &str = "id: ";
/// The field that carries a frame's data.
pub(crate) const DATA_FIELD: &str = "data: ";
/// What ends one line of a block.
pub(crate) const LINE_END: &str = "\n";
/// What ends a block of the stream.
pub(crate) const BLOCK_END: &str = "\n\n";
/// What opens a comment line, which a page ignores.
const COMMENT_FIELD: &str = ": ";

/// One Server-Sent Events frame: `kind` as its event, `id` as its id and
/// `data`, one line, as its data.
#[must_use]
pub fn event_frame(kind: &str, id: u64, data: &str) -> String {
    block(&format!(
        "{EVENT_FIELD}{kind}{LINE_END}{ID_FIELD}{id}{LINE_END}{DATA_FIELD}{data}"
    ))
}

/// The block telling a page to wait `millis` before reconnecting.
#[must_use]
pub fn retry_frame(millis: u64) -> String {
    block(&format!("{RETRY_FIELD}{millis}"))
}

/// The block holding only the comment `text`.
#[must_use]
pub fn comment_frame(text: &str) -> String {
    block(&format!("{COMMENT_FIELD}{text}"))
}

/// `lines` ended as one block of the stream.
fn block(lines: &str) -> String {
    format!("{lines}{BLOCK_END}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An event frame is its event, id and data lines, ended as one block.
    #[test]
    fn an_event_frame_is_three_lines_in_one_block() {
        assert_eq!(
            event_frame("speaking", 7, "{}"),
            "event: speaking\nid: 7\ndata: {}\n\n"
        );
        assert_eq!(retry_frame(1_000), "retry: 1000\n\n");
        assert_eq!(comment_frame("keepalive"), ": keepalive\n\n");
    }
}
