use strum::{Display, EnumIter, EnumString, IntoEnumIterator};

/// Opens the hook's matcher: a whole source, nothing around it.
const MATCHER_OPEN: &str = "^(";
/// Between the sources the matcher takes.
const MATCHER_OR: &str = "|";
/// Closes the hook's matcher.
const MATCHER_CLOSE: &str = ")$";

/// How an agent's session starts, as its session-start hook is told: each
/// starts from a fresh context, so each runs muster's hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Display, EnumString, EnumIter)]
#[strum(serialize_all = "kebab-case")]
pub enum SessionSource {
    /// A new session.
    Startup,
    /// A session resumed.
    Resume,
    /// A session cleared.
    Clear,
    /// A session compacted.
    Compact,
}

impl SessionSource {
    /// The hook's matcher, taking every source whole and nothing else.
    pub fn matcher() -> String {
        let sources: Vec<String> = Self::iter().map(|source| source.to_string()).collect();
        format!("{MATCHER_OPEN}{}{MATCHER_CLOSE}", sources.join(MATCHER_OR))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The matcher names each source as it parses back, anchored whole.
    #[test]
    fn the_matcher_names_each_source_whole() {
        assert_eq!(SessionSource::matcher(), "^(startup|resume|clear|compact)$");
        for source in SessionSource::iter() {
            assert_eq!(source.to_string().parse::<SessionSource>().unwrap(), source);
        }
    }
}
