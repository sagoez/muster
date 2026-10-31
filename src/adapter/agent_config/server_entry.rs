use std::path::Path;

use getset::Getters;
use jsonc_parser::cst::CstInputValue;
use serde_json::Value;
use typed_builder::TypedBuilder;

use super::entry_match::EntryMatch;
use crate::constants::APP_NAME;

/// The key of an entry's program.
pub(super) const COMMAND_KEY: &str = "command";
/// The key of an entry's arguments.
pub(super) const ARGS_KEY: &str = "args";
/// The key of an entry's environment, which Zed's entries carry.
const ENV_KEY: &str = "env";
/// The key that turns an entry off when false, as Codex and Zed read it.
pub(super) const ENABLED_KEY: &str = "enabled";

/// muster's server entry as one config file writes it.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
pub(super) struct ServerEntry {
    /// The program the agent runs.
    #[getset(get = "pub(super)")]
    command: String,
    /// Its arguments.
    #[getset(get = "pub(super)")]
    args: Vec<String>,
    /// Whether the entry carries an empty `env`, as Zed's documented form
    /// does.
    with_env: bool,
}

impl ServerEntry {
    /// The entry as a JSON value to insert.
    pub(super) fn json_input(&self) -> CstInputValue {
        let mut fields = vec![
            (
                COMMAND_KEY.to_owned(),
                CstInputValue::from(self.command.as_str()),
            ),
            (
                ARGS_KEY.to_owned(),
                CstInputValue::Array(
                    self.args
                        .iter()
                        .map(|arg| CstInputValue::from(arg.as_str()))
                        .collect(),
                ),
            ),
        ];
        if self.with_env {
            fields.push((ENV_KEY.to_owned(), CstInputValue::Object(Vec::new())));
        }
        CstInputValue::Object(fields)
    }

    /// What `value`, an existing JSON entry, is to muster, and whether its
    /// own flag turns it off.
    pub(super) fn judge_json(&self, value: &Value) -> (EntryMatch, bool) {
        let matched = self.judge(
            value.get(COMMAND_KEY).and_then(Value::as_str),
            value
                .get(ARGS_KEY)
                .and_then(Value::as_array)
                .map(|args| args.iter().map(Value::as_str).collect()),
        );
        let off = turned_off(value.get(ENABLED_KEY).and_then(Value::as_bool));
        (matched, off)
    }

    /// What an existing entry holding `command` and `args` is to muster:
    /// the same entry, muster's own at another path (a program named
    /// `muster`, with the same arguments), or another server's. A binary
    /// run under another name knows only the entry it writes itself, so a
    /// generic name such as `server` is never claimed.
    pub(super) fn judge(
        &self,
        command: Option<&str>,
        args: Option<Vec<Option<&str>>>,
    ) -> EntryMatch {
        let same_args = args.is_some_and(|args| {
            args.into_iter()
                .eq(self.args.iter().map(|arg| Some(arg.as_str())))
        });
        let musters = |command: &str| Path::new(command).file_name() == Some(APP_NAME.as_ref());
        match command {
            Some(command) if same_args && command == self.command => EntryMatch::Same,
            Some(command) if same_args && musters(command) => EntryMatch::Older,
            _ => EntryMatch::Foreign,
        }
    }
}

/// Whether an entry whose `enabled` flag is as given is turned off: only an
/// explicit `false` turns it off.
pub(super) fn turned_off(enabled: Option<bool>) -> bool {
    enabled == Some(false)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::adapter::agent_config::testing::entry;

    /// The same command is the same entry, a `muster` elsewhere with the
    /// same arguments is muster's older one, and anything else is another
    /// server's; only `enabled: false` turns one off, as no agent reads a
    /// `disabled` key.
    #[test]
    fn an_entry_is_judged_by_its_program_and_arguments() {
        let ours = entry(false);
        for (value, expected) in [
            (
                json!({"command": "/bin/muster", "args": ["mcp"]}),
                (EntryMatch::Same, false),
            ),
            (
                json!({"command": "/old/0.9/bin/muster", "args": ["mcp"], "enabled": false}),
                (EntryMatch::Older, true),
            ),
            (
                json!({"command": "muster", "args": ["mcp"], "disabled": true}),
                (EntryMatch::Older, false),
            ),
            (
                json!({"command": "/bin/muster", "args": ["mcp", "--as", "x"]}),
                (EntryMatch::Foreign, false),
            ),
            (
                json!({"command": "/bin/muster-dev", "args": ["mcp"]}),
                (EntryMatch::Foreign, false),
            ),
            (
                json!({"command": "/bin/muster"}),
                (EntryMatch::Foreign, false),
            ),
            (json!(3), (EntryMatch::Foreign, false)),
        ] {
            assert_eq!(ours.judge_json(&value), expected, "{value}");
        }
    }

    /// A binary run under a generic name such as `server` claims no other
    /// `server` with the arguments `mcp`, only `muster` elsewhere and the
    /// entry it writes itself.
    #[test]
    fn a_generic_name_is_never_claimed() {
        let renamed = ServerEntry::builder()
            .command("/opt/tools/server".to_owned())
            .args(vec!["mcp".to_owned()])
            .with_env(false)
            .build();
        for (command, expected) in [
            ("/usr/bin/server", EntryMatch::Foreign),
            ("server", EntryMatch::Foreign),
            ("/bin/muster", EntryMatch::Older),
            ("/opt/tools/server", EntryMatch::Same),
        ] {
            let value = json!({"command": command, "args": ["mcp"]});
            assert_eq!(renamed.judge_json(&value).0, expected, "{command}");
        }
        let generic = json!({"command": "/usr/bin/server", "args": ["mcp"]});
        assert_eq!(entry(false).judge_json(&generic).0, EntryMatch::Foreign);
    }
}
