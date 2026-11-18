use strum::{EnumProperty, EnumString};

/// The property saying a command publishes the checkout.
const PUBLISHES: &str = "publishes";

/// A `gh pr` subcommand that publishes a checkout or lands it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumString, EnumProperty)]
#[strum(serialize_all = "kebab-case")]
pub enum PrCommand {
    /// `gh pr create`.
    #[strum(props(publishes = true))]
    Create,
    /// `gh pr merge`.
    Merge,
}

impl PrCommand {
    /// Whether it publishes the checkout, after which its todo should carry
    /// the pull request.
    pub fn publishes(self) -> bool {
        self.get_bool(PUBLISHES).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    /// A create publishes, a merge does not, and a view is no such command.
    #[test]
    fn only_a_create_publishes() {
        assert!(PrCommand::Create.publishes());
        assert!(!PrCommand::Merge.publishes());
        assert!(PrCommand::from_str("view").is_err());
    }
}
