use std::sync::LazyLock;

use nutype::nutype;

use crate::adapter::spoken_list::{Conjunction, spoken_list};

/// Most characters a server name holds, as Claude Code allows.
const SERVER_NAME_MAX_CHARS: usize = 64;
/// The name muster's entry takes unless another is asked for, and the key
/// `muster mcp --print-config` writes.
pub const DEFAULT_SERVER_NAME: &str = "muster";
/// The punctuation a server name may hold besides ASCII letters and digits.
const NAME_PUNCTUATION: [char; 2] = ['-', '_'];
/// The kinds of character a server name holds besides its punctuation.
const NAME_KINDS: [&str; 2] = ["ASCII letters", "digits"];

/// The form a server name takes, for a refusal of one that does not.
pub static SERVER_NAME_FORM: LazyLock<String> = LazyLock::new(|| {
    let kinds: Vec<String> = NAME_KINDS
        .iter()
        .map(ToString::to_string)
        .chain(NAME_PUNCTUATION.iter().map(|c| format!("'{c}'")))
        .collect();
    format!(
        "use 1 to {SERVER_NAME_MAX_CHARS} {}",
        spoken_list(&kinds, Conjunction::And)
    )
});

/// Whether `name` is a key every supported agent accepts for a server.
fn is_server_name(name: &str) -> bool {
    (1..=SERVER_NAME_MAX_CHARS).contains(&name.chars().count())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || NAME_PUNCTUATION.contains(&c))
}

/// The key an MCP server entry goes under, in the form
/// [`SERVER_NAME_FORM`] describes.
#[nutype(
    validate(predicate = is_server_name),
    derive(Debug, Clone, PartialEq, Eq, Hash, AsRef, Display, Serialize)
)]
pub struct McpServerName(String);

#[cfg(test)]
mod tests {
    use super::*;

    /// Only names every agent takes as a key are accepted.
    #[test]
    fn a_server_name_is_a_plain_key() {
        for good in [DEFAULT_SERVER_NAME, "my_muster-2"] {
            assert!(McpServerName::try_new(good).is_ok(), "{good}");
        }
        let long = "m".repeat(SERVER_NAME_MAX_CHARS + 1);
        for bad in ["", "a b", "a.b", "a/b", "ü", long.as_str()] {
            assert!(McpServerName::try_new(bad).is_err(), "{bad}");
        }
        assert_eq!(
            *SERVER_NAME_FORM,
            "use 1 to 64 ASCII letters, digits, '-' and '_'"
        );
    }
}
