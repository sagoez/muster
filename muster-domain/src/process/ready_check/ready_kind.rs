use strum::{Display, EnumIter, EnumString};

/// How a readiness check is stored: the kind column of a process row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumString, EnumIter)]
#[strum(serialize_all = "kebab-case")]
pub enum ReadyKind {
    /// A substring of one output line.
    Log,
    /// A regex over one output line.
    Regex,
    /// A TCP connect to a local port.
    Port,
    /// An HTTP response below 500 from a local URL.
    Http,
}
