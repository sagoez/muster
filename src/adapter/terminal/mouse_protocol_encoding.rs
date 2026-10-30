/// Byte encoding a child expects for mouse reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseProtocolEncoding {
    /// Legacy single-byte encoding.
    Default,
    /// UTF-8 extended coordinates (mode 1005).
    Utf8,
    /// SGR encoding (mode 1006).
    Sgr,
}
