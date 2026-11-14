use regex::Regex;

/// How a compiled matcher tests a line.
#[derive(Debug, Clone)]
pub(super) enum Matcher {
    /// The line contains the text.
    Substring(String),
    /// The regex is found in the line.
    Regex(Regex),
}
