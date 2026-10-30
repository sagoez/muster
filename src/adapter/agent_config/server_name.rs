use nutype::nutype;

/// Most characters a server name holds, as Claude Code allows.
pub const SERVER_NAME_MAX_CHARS: usize = 64;
/// The name muster's entry takes unless another is asked for.
pub const DEFAULT_SERVER_NAME: &str = "muster";
/// The punctuation a server name may hold besides ASCII letters and digits.
const NAME_PUNCTUATION: [char; 2] = ['-', '_'];

/// Whether `name` is a key every supported agent accepts for a server.
fn is_server_name(name: &str) -> bool {
    (1..=SERVER_NAME_MAX_CHARS).contains(&name.chars().count())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || NAME_PUNCTUATION.contains(&c))
}

/// The key an MCP server entry goes under: 1 to [`SERVER_NAME_MAX_CHARS`]
/// ASCII letters, digits, `-` and `_`.
#[nutype(
    validate(predicate = is_server_name),
    derive(Debug, Clone, PartialEq, Eq, Hash, AsRef, Display, FromStr, Serialize)
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
    }
}
