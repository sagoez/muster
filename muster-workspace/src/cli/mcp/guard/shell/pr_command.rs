use strum::EnumString;

/// A `gh pr` subcommand that switches, publishes or lands a checkout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumString)]
#[strum(serialize_all = "kebab-case")]
pub enum PrCommand {
    /// `gh pr create`.
    Create,
    /// `gh pr merge`.
    Merge,
    /// `gh pr checkout`, which switches the branch.
    Checkout,
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    /// The guarded subcommands parse, and a view is no such command.
    #[test]
    fn guarded_subcommands_parse() {
        assert_eq!(PrCommand::from_str("checkout"), Ok(PrCommand::Checkout));
        assert!(PrCommand::from_str("view").is_err());
    }
}
