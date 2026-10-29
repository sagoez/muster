use std::path::Path;

use super::{candidate::Candidate, detect_source::DetectSource};
use crate::{
    adapter::bounded_read::read_bounded,
    domain::{
        config::OverCap,
        value::{CommandLine, ProcessName},
    },
};

/// Largest file a detector reads; anything bigger is not a stack definition
/// worth guessing from.
const MAX_SOURCE_BYTES: u64 = 1024 * 1024;

/// The text of the first of `names` under `root` that is a regular file no
/// larger than [`MAX_SOURCE_BYTES`].
pub fn read_first(root: &Path, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| read_source(&root.join(name)))
}

/// The text of each of `names` under `root` that can be read, in order.
pub fn read_each(root: &Path, names: &[&str]) -> Vec<String> {
    names
        .iter()
        .filter_map(|name| read_source(&root.join(name)))
        .collect()
}

/// Whether any of `names` under `root` is a file.
pub fn any_exists(root: &Path, names: &[&str]) -> bool {
    names.iter().any(|name| root.join(name).is_file())
}

/// The text of `path` when it is a regular UTF-8 file no larger than
/// [`MAX_SOURCE_BYTES`].
fn read_source(path: &Path) -> Option<String> {
    read_bounded(path, MAX_SOURCE_BYTES).ok()
}

/// A candidate named `name` running `command`, or none when either is not
/// usable as a new process name or command.
pub fn candidate(
    name: &str,
    command: &str,
    source: DetectSource,
    suggested: bool,
) -> Option<Candidate> {
    Some(
        Candidate::builder()
            .name(new_name(name)?)
            .command(CommandLine::try_new(command).ok()?)
            .source(source)
            .suggested(suggested)
            .build(),
    )
}

/// `raw` as the name of a new process, or none when it is not one or runs
/// past the name cap a write would refuse.
pub fn new_name(raw: &str) -> Option<ProcessName> {
    ProcessName::try_new(raw)
        .ok()
        .filter(|name| OverCap::of_name(name).is_none())
}

/// Whether `name` is one of `words`, or starts with one of `prefixes`.
pub fn suggested(name: &str, words: &[&str], prefixes: &[&str]) -> bool {
    words.contains(&name) || prefixes.iter().any(|prefix| name.starts_with(prefix))
}

/// `word` quoted for a shell when it needs it.
pub fn shell_word(word: &str) -> String {
    shlex::try_quote(word)
        .map(|quoted| quoted.into_owned())
        .unwrap_or_else(|_| word.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::value::PROCESS_NAME_MAX_CHARS;

    /// A script named past the name cap is never offered, since adding it
    /// would be refused; one at the cap is.
    #[test]
    fn a_name_past_the_cap_is_never_a_candidate() {
        let named =
            |chars: usize| candidate(&"n".repeat(chars), "true", DetectSource::PackageJson, false);

        assert!(named(PROCESS_NAME_MAX_CHARS).is_some());
        assert!(named(PROCESS_NAME_MAX_CHARS + 1).is_none());
    }
}
