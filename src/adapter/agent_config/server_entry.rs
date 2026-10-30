use getset::{CopyGetters, Getters};
use jsonc_parser::cst::CstInputValue;
use serde_json::Value;
use typed_builder::TypedBuilder;

/// The key of an entry's program.
pub(super) const COMMAND_KEY: &str = "command";
/// The key of an entry's arguments.
pub(super) const ARGS_KEY: &str = "args";
/// The key of an entry's environment, which Zed's entries carry.
const ENV_KEY: &str = "env";

/// muster's server entry as one config file writes it.
#[derive(Clone, Debug, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub(super) struct ServerEntry {
    /// The program the agent runs.
    #[getset(get = "pub(super)")]
    command: String,
    /// Its arguments.
    #[getset(get = "pub(super)")]
    args: Vec<String>,
    /// Whether the entry carries an empty `env`, as Zed's documented form
    /// does.
    #[getset(get_copy = "pub(super)")]
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

    /// Whether `value`, an existing entry, runs the same program with the
    /// same arguments; keys it holds besides those are its own business.
    pub(super) fn matches_json(&self, value: &Value) -> bool {
        self.matches(
            value.get(COMMAND_KEY).and_then(Value::as_str),
            value
                .get(ARGS_KEY)
                .and_then(Value::as_array)
                .map(|args| args.iter().map(Value::as_str).collect()),
        )
    }

    /// Whether an existing entry holding `command` and `args` is this one.
    pub(super) fn matches(&self, command: Option<&str>, args: Option<Vec<Option<&str>>>) -> bool {
        command == Some(self.command.as_str())
            && args.is_some_and(|args| {
                args.into_iter()
                    .eq(self.args.iter().map(|arg| Some(arg.as_str())))
            })
    }
}
