//! What a command's name reads as in muster's own text.

use muster_domain::constants::APP_NAME;

use crate::cli::command_line::command::CommandName;

impl CommandName {
    /// The command as a person types it, such as `muster logs`.
    pub fn invocation(self) -> String {
        format!("{APP_NAME} {self}")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use clap::{Command as ClapCommand, CommandFactory};
    use strum::IntoEnumIterator;

    use super::*;
    use crate::cli::command_line::args::Args;

    /// Every command's name is the one clap parses, hidden ones included,
    /// and a command reads as muster then its name.
    #[test]
    fn every_name_is_the_one_clap_parses() {
        let named: BTreeSet<String> = CommandName::iter().map(|name| name.to_string()).collect();
        let parsed: BTreeSet<String> = Args::command()
            .get_subcommands()
            .map(ClapCommand::get_name)
            .map(str::to_string)
            .collect();

        assert_eq!(named, parsed);
        assert_eq!(CommandName::Logs.invocation(), "muster logs");
        assert_eq!(CommandName::ShellInit.invocation(), "muster shell-init");
    }
}
