use std::path::Path;

use super::{candidate::Candidate, detect_source::DetectSource};
use crate::{
    adapter::bounded_read::read_bounded,
    domain::value::{CommandLine, ProcessName},
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
/// usable as a process name or command.
pub fn candidate(
    name: &str,
    command: &str,
    source: DetectSource,
    suggested: bool,
) -> Option<Candidate> {
    Some(
        Candidate::builder()
            .name(ProcessName::try_new(name).ok()?)
            .command(CommandLine::try_new(command).ok()?)
            .source(source)
            .suggested(suggested)
            .build(),
    )
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
