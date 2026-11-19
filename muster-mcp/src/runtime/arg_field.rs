//! Every argument a tool refuses by name: the one source of the field names a
//! refusal states and `process_set` reads by hand.

use std::sync::LazyLock;

use strum::{AsRefStr, Display, EnumIter, EnumProperty, IntoEnumIterator, IntoStaticStr};

/// The property marking a field `process_set` takes.
const PROCESS_SET: &str = "process_set";

/// The fields `process_set` takes, in the order a refusal of an unknown
/// field lists them.
pub(crate) static PROCESS_SET_FIELDS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    ArgField::iter()
        .filter(|field| field.taken_by_process_set())
        .map(<&'static str>::from)
        .collect()
});

/// An argument of a tool, spelled as agents pass it. MCP clients read the
/// argument names from the input schema, which names them after the
/// snake_case fields of the argument structs, so each variant carries its
/// snake_case spelling, not kebab-case.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Display, AsRefStr, EnumIter, EnumProperty, IntoStaticStr,
)]
pub enum ArgField {
    /// The project a call acts on.
    #[strum(serialize = "project", props(process_set = true))]
    Project,
    /// A process's or a lock's name.
    #[strum(serialize = "name", props(process_set = true))]
    Name,
    /// A process's shell command.
    #[strum(serialize = "command", props(process_set = true))]
    Command,
    /// Where a process runs.
    #[strum(serialize = "working_dir", props(process_set = true))]
    WorkingDir,
    /// A process's one-line note.
    #[strum(serialize = "description", props(process_set = true))]
    Description,
    /// The icon `muster view` draws a process with.
    #[strum(serialize = "icon", props(process_set = true))]
    Icon,
    /// The environment variables `process_set` adds or replaces.
    #[strum(serialize = "environment_set", props(process_set = true))]
    EnvironmentSet,
    /// The environment variables `process_set` removes.
    #[strum(serialize = "environment_unset", props(process_set = true))]
    EnvironmentUnset,
    /// Whether `muster up` starts a process.
    #[strum(serialize = "autostart", props(process_set = true))]
    Autostart,
    /// A process's restart policy.
    #[strum(serialize = "restart", props(process_set = true))]
    Restart,
    /// The signal a stop sends.
    #[strum(serialize = "stop_signal", props(process_set = true))]
    StopSignal,
    /// How long a stop waits before it kills.
    #[strum(serialize = "stop_grace", props(process_set = true))]
    StopGrace,
    /// A process's readiness check.
    #[strum(serialize = "ready", props(process_set = true))]
    Ready,
    /// The TCP ports a process listens on.
    #[strum(serialize = "ports", props(process_set = true))]
    Ports,
    /// The environment variables of a new process.
    #[strum(serialize = "environment")]
    Environment,
    /// A note's or a value's key, or a setting's.
    #[strum(serialize = "key")]
    Key,
    /// A note's body.
    #[strum(serialize = "body")]
    Body,
    /// A key-value entry's value.
    #[strum(serialize = "value")]
    Value,
    /// A todo's id.
    #[strum(serialize = "id")]
    Id,
    /// A todo's title.
    #[strum(serialize = "title")]
    Title,
    /// A todo's status.
    #[strum(serialize = "status")]
    Status,
    /// The todos a todo waits on.
    #[strum(serialize = "deps")]
    Deps,
    /// Hands a todo back.
    #[strum(serialize = "release")]
    Release,
    /// A lock's note.
    #[strum(serialize = "note")]
    Note,
    /// How long a todo claim lasts.
    #[strum(serialize = "lease_s")]
    LeaseS,
    /// How long a lock lasts.
    #[strum(serialize = "ttl_s")]
    TtlS,
    /// Where a page of entries starts.
    #[strum(serialize = "cursor")]
    Cursor,
    /// The most entries a page lists.
    #[strum(serialize = "limit")]
    Limit,
    /// How long a call waits for its condition.
    #[strum(serialize = "timeout_s")]
    TimeoutS,
    /// How long a call waits for a lock or an event.
    #[strum(serialize = "wait_s")]
    WaitS,
    /// What a control call waits for.
    #[strum(serialize = "wait")]
    Wait,
    /// What `process_wait` waits for.
    #[strum(serialize = "until")]
    Until,
    /// The output a read starts after.
    #[strum(serialize = "since")]
    Since,
    /// The text a log read keeps.
    #[strum(serialize = "grep")]
    Grep,
    /// Reads grep as a regex.
    #[strum(serialize = "regex")]
    Regex,
    /// The most lines a log read returns.
    #[strum(serialize = "lines")]
    Lines,
    /// The most bytes a log read returns.
    #[strum(serialize = "max_bytes")]
    MaxBytes,
    /// The event kinds a read keeps.
    #[strum(serialize = "kinds")]
    Kinds,
    /// The way a pane splits.
    #[strum(serialize = "direction")]
    Direction,
}

impl ArgField {
    /// Whether `process_set` takes this field.
    pub(crate) fn taken_by_process_set(self) -> bool {
        self.get_bool(PROCESS_SET).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::runtime::{
        ToolName,
        test_support::{arguments_of_tool, fixture},
    };

    /// Every field is an argument some tool takes, and `process_set` takes
    /// exactly the fields marked as its own.
    #[test]
    fn every_field_is_an_argument_a_tool_takes() {
        let fixture = fixture();
        let taken: BTreeSet<String> = fixture
            .server()
            .tool_router
            .list_all()
            .iter()
            .flat_map(arguments_of_tool)
            .collect();

        for field in ArgField::iter() {
            assert!(taken.contains(field.as_ref()), "{field}");
        }
        let marked: BTreeSet<String> = PROCESS_SET_FIELDS.iter().map(ToString::to_string).collect();
        assert_eq!(
            marked,
            arguments_of_tool(&fixture.tool(ToolName::ProcessSet))
        );
    }
}
