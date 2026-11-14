use strum::{Display, EnumString};

/// How a log pattern is matched against one output line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display, EnumString)]
#[strum(serialize_all = "kebab-case")]
pub enum PatternKind {
    /// The line contains the text.
    Substring,
    /// The line matches the text as a regular expression.
    Regex,
}

impl PatternKind {
    /// A regex when `regex`, else a substring.
    pub fn of_regex(regex: bool) -> Self {
        if regex { Self::Regex } else { Self::Substring }
    }
}
