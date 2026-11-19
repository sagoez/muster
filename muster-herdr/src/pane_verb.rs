//! The `herdr pane` subcommands muster calls, each with what its call is
//! called in an error.

use std::{ffi::OsString, fmt::Display};

use strum::{AsRefStr, EnumIter, EnumMessage};

/// herdr's command group for panes.
const PANE_GROUP: &str = "pane";

/// A `herdr pane` subcommand. Its message is what the call is called in an
/// error, before the pane it acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, AsRefStr, EnumIter, EnumMessage)]
#[strum(serialize_all = "kebab-case")]
pub(crate) enum PaneVerb {
    /// Reads a tab's layout.
    #[strum(message = "read the layout of pane")]
    Layout,
    /// Opens a pane beside another.
    #[strum(message = "split pane")]
    Split,
    /// Types a line and Enter into a pane.
    #[strum(message = "run muster in pane")]
    Run,
    /// Closes a pane.
    #[strum(message = "close pane")]
    Close,
    /// Describes a pane.
    #[strum(message = "describe pane")]
    Get,
    /// Focuses the pane on one side of another.
    #[strum(message = "focus the new pane beside pane")]
    Focus,
}

impl PaneVerb {
    /// The arguments of this call: `pane <verb>`, then `rest`.
    pub(crate) fn args(self, rest: &[&str]) -> Vec<OsString> {
        [PANE_GROUP, self.as_ref()]
            .iter()
            .chain(rest)
            .map(OsString::from)
            .collect()
    }

    /// What this call on `pane` is called in an error: `split pane w1:p1`.
    pub(crate) fn action(self, pane: impl Display) -> String {
        format!("{} {pane}", self.get_message().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;

    /// Each verb names its subcommand in kebab case and its action in words.
    #[test]
    fn each_verb_has_its_subcommand_and_its_action() {
        let calls: Vec<(Vec<OsString>, String)> = PaneVerb::iter()
            .map(|verb| (verb.args(&["w1:p1"]), verb.action("w1:p1")))
            .collect();
        let expected = [
            ("layout", "read the layout of pane w1:p1"),
            ("split", "split pane w1:p1"),
            ("run", "run muster in pane w1:p1"),
            ("close", "close pane w1:p1"),
            ("get", "describe pane w1:p1"),
            ("focus", "focus the new pane beside pane w1:p1"),
        ];
        assert_eq!(calls.len(), expected.len());
        for ((args, action), (verb, words)) in calls.iter().zip(expected) {
            assert_eq!(args, &["pane", verb, "w1:p1"].map(OsString::from));
            assert_eq!(action, words);
        }
    }
}
