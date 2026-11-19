//! The placeholders a tool description, an argument description or the
//! server instructions may hold, each standing for the value of the constant
//! or enum that enforces it.

use muster_client::{DEFAULT_LIST_LIMIT, LISTED_VALUE_CHARS, MAX_LIST_LIMIT};
use muster_domain::{
    config::{DECLARED_PORTS_MAX, ENV_VARS_MAX, PROCESSES_MAX},
    coordination::{
        KV_KEY_MAX_CHARS, KV_VALUE_MAX_CHARS, LOCK_NAME_MAX_CHARS, LOCK_TTL_DEFAULT, LOCK_TTL_MAX,
        LOCKS_PER_SESSION_MAX, NOTE_BODY_MAX_CHARS, NOTE_KEY_MAX_CHARS, TODO_DEPS_MAX,
        TODO_LEASE_DEFAULT, TODO_LEASE_MAX, TODO_TITLE_MAX_CHARS, TodoStatus,
    },
    process::{ProcessIcon, RestartPolicy, StopSignal},
    terminal_workspace::{MIN_HALF_COLS, SplitDirection},
    value::{
        COMMAND_MAX_CHARS, DESCRIPTION_MAX_CHARS, ENV_NAME_MAX_CHARS, ENV_VALUE_MAX_CHARS,
        MAX_STOP_GRACE, PROCESS_NAME_MAX_CHARS,
    },
};
use muster_protocol::{
    holder::{DEFAULT_LOG_BYTES, DEFAULT_LOG_LINES, MAX_LOG_BYTES, MAX_LOG_LINES},
    service::{EVENTS_WITHOUT_SINCE, EventKind, FAILURE_TAIL_LINES},
};
use strum::{Display, EnumIter};

use super::{
    bounds::{CALL_BOUND, DEFAULT_WAIT},
    choices::Choices,
};
use crate::process::lifecycle::condition_field::ConditionField;

/// What the shared writes and the server instructions say of the project
/// they need, so they word it alike.
const NEEDS_PROJECT: &str = "Notes, todos, values and locks need a registered project and never \
    register one: in a folder that is not a project yet, call project_register first, which \
    needs no process.";

/// What the server instructions say muster's plugin holds an agent to
/// before it edits or runs git, worded so it holds where no plugin runs.
pub(super) const GUARDED_WORK: &str = "Before editing a project's files, claim a todo for the \
    work (todo_claim), and before git commands that change or publish a checkout, also take the \
    checkout's lock (lock_acquire); where muster's plugin is installed it refuses those calls \
    until you do, naming the exact call to make.";

/// A placeholder, written in a description as its kebab-case name in angle
/// brackets, such as `<call-bound-s>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Display, EnumIter)]
#[strum(serialize_all = "kebab-case", prefix = "<", suffix = ">")]
pub(super) enum Placeholder {
    /// That notes, todos, values and locks need a registered project.
    NeedsProject,
    /// What the plugin refuses until a todo is claimed or a lock taken.
    GuardedWork,
    /// Every process icon.
    ProcessIcons,
    /// Every stack event kind.
    EventKinds,
    /// Every split direction.
    SplitDirections,
    /// Every todo status, as alternatives in a sentence.
    TodoStatuses,
    /// Every restart policy, piped.
    RestartPolicies,
    /// Every restart policy, quoted alternatives in a sentence.
    RestartPolicyChoices,
    /// Every stop signal, piped.
    StopSignals,
    /// Every stop signal with the Unix signal it sends, quoted alternatives
    /// in a sentence.
    StopSignalChoices,
    /// Every kind of wait condition, quoted alternatives in a sentence.
    ConditionKinds,
    /// The fewest columns each half of a split keeps.
    SplitMinHalfCols,
    /// The longest process name.
    ProcessNameMaxChars,
    /// The most processes a stack holds.
    ProcessesMax,
    /// The most environment variables a process holds.
    EnvVarsMax,
    /// The most ports a process declares.
    DeclaredPortsMax,
    /// The longest environment variable name.
    EnvNameMaxChars,
    /// The longest environment variable value.
    EnvValueMaxChars,
    /// The most locks one session holds.
    LocksPerSessionMax,
    /// The longest stop grace, in seconds.
    MaxStopGraceS,
    /// How much of a listed value is shown.
    ListedValueChars,
    /// The longest note key.
    NoteKeyMaxChars,
    /// The longest note body.
    NoteBodyMaxChars,
    /// The longest value key.
    KvKeyMaxChars,
    /// The longest value.
    KvValueMaxChars,
    /// The longest todo title.
    TodoTitleMaxChars,
    /// The most dependencies a todo names.
    TodoDepsMax,
    /// The longest command.
    CommandMaxChars,
    /// The longest process description.
    DescriptionMaxChars,
    /// The longest lock name.
    LockNameMaxChars,
    /// The longest a call blocks, in seconds.
    CallBoundS,
    /// How long a wait runs by default, in seconds.
    DefaultWaitS,
    /// How many lines a failure carries.
    FailureTailLines,
    /// How many log lines a read returns by default.
    DefaultLogLines,
    /// The most log lines a read returns.
    MaxLogLines,
    /// How many log bytes a read returns by default.
    DefaultLogBytes,
    /// The most log bytes a read returns.
    MaxLogBytes,
    /// A todo claim's default lease, in seconds.
    DefaultLeaseS,
    /// A todo claim's longest lease, in seconds.
    MaxLeaseS,
    /// A lock's default TTL, in seconds.
    DefaultTtlS,
    /// A lock's longest TTL, in seconds.
    MaxTtlS,
    /// How many entries a page lists by default.
    DefaultListLimit,
    /// The most entries a page lists.
    MaxListLimit,
    /// How many events a read without since returns.
    EventsWithoutSince,
}

impl Placeholder {
    /// The text this placeholder stands for.
    pub(super) fn value(self) -> String {
        match self {
            Self::NeedsProject => NEEDS_PROJECT.to_string(),
            Self::GuardedWork => GUARDED_WORK.to_string(),
            Self::ProcessIcons => Choices::of::<ProcessIcon>().listed(),
            Self::EventKinds => Choices::of::<EventKind>().listed(),
            Self::SplitDirections => Choices::of::<SplitDirection>().listed(),
            Self::TodoStatuses => Choices::of::<TodoStatus>().either(),
            Self::RestartPolicies => Choices::of::<RestartPolicy>().piped(),
            Self::RestartPolicyChoices => Choices::of::<RestartPolicy>().coded().either(),
            Self::StopSignals => Choices::of::<StopSignal>().piped(),
            Self::StopSignalChoices => Choices::glossed(StopSignal::unix_name).either(),
            Self::ConditionKinds => Choices::of_values(ConditionField::kinds()).coded().either(),
            Self::SplitMinHalfCols => MIN_HALF_COLS.to_string(),
            Self::ProcessNameMaxChars => PROCESS_NAME_MAX_CHARS.to_string(),
            Self::ProcessesMax => PROCESSES_MAX.to_string(),
            Self::EnvVarsMax => ENV_VARS_MAX.to_string(),
            Self::DeclaredPortsMax => DECLARED_PORTS_MAX.to_string(),
            Self::EnvNameMaxChars => ENV_NAME_MAX_CHARS.to_string(),
            Self::EnvValueMaxChars => ENV_VALUE_MAX_CHARS.to_string(),
            Self::LocksPerSessionMax => LOCKS_PER_SESSION_MAX.to_string(),
            Self::MaxStopGraceS => MAX_STOP_GRACE.as_secs().to_string(),
            Self::ListedValueChars => LISTED_VALUE_CHARS.to_string(),
            Self::NoteKeyMaxChars => NOTE_KEY_MAX_CHARS.to_string(),
            Self::NoteBodyMaxChars => NOTE_BODY_MAX_CHARS.to_string(),
            Self::KvKeyMaxChars => KV_KEY_MAX_CHARS.to_string(),
            Self::KvValueMaxChars => KV_VALUE_MAX_CHARS.to_string(),
            Self::TodoTitleMaxChars => TODO_TITLE_MAX_CHARS.to_string(),
            Self::TodoDepsMax => TODO_DEPS_MAX.to_string(),
            Self::CommandMaxChars => COMMAND_MAX_CHARS.to_string(),
            Self::DescriptionMaxChars => DESCRIPTION_MAX_CHARS.to_string(),
            Self::LockNameMaxChars => LOCK_NAME_MAX_CHARS.to_string(),
            Self::CallBoundS => CALL_BOUND.as_secs().to_string(),
            Self::DefaultWaitS => DEFAULT_WAIT.as_secs().to_string(),
            Self::FailureTailLines => FAILURE_TAIL_LINES.to_string(),
            Self::DefaultLogLines => DEFAULT_LOG_LINES.to_string(),
            Self::MaxLogLines => MAX_LOG_LINES.to_string(),
            Self::DefaultLogBytes => DEFAULT_LOG_BYTES.to_string(),
            Self::MaxLogBytes => MAX_LOG_BYTES.to_string(),
            Self::DefaultLeaseS => TODO_LEASE_DEFAULT.as_secs().to_string(),
            Self::MaxLeaseS => TODO_LEASE_MAX.as_secs().to_string(),
            Self::DefaultTtlS => LOCK_TTL_DEFAULT.as_secs().to_string(),
            Self::MaxTtlS => LOCK_TTL_MAX.as_secs().to_string(),
            Self::DefaultListLimit => DEFAULT_LIST_LIMIT.to_string(),
            Self::MaxListLimit => MAX_LIST_LIMIT.to_string(),
            Self::EventsWithoutSince => EVENTS_WITHOUT_SINCE.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A placeholder is written as its kebab-case name in angle brackets,
    /// and a list reads from its enum.
    #[test]
    fn a_placeholder_is_its_name_in_angle_brackets() {
        assert_eq!(Placeholder::MaxStopGraceS.to_string(), "<max-stop-grace-s>");
        assert_eq!(Placeholder::KvKeyMaxChars.to_string(), "<kv-key-max-chars>");
        assert_eq!(
            Placeholder::TodoStatuses.value(),
            "pending, in-progress or done"
        );
        assert_eq!(
            Placeholder::RestartPolicyChoices.value(),
            "`never`, `on-failure` or `always`"
        );
        assert_eq!(Placeholder::StopSignals.value(), "terminate|interrupt");
        assert_eq!(
            Placeholder::StopSignalChoices.value(),
            "`terminate` (SIGTERM) or `interrupt` (SIGINT)"
        );
        assert_eq!(
            Placeholder::ConditionKinds.value(),
            "`log`, `port`, `http`, `state` or `ready`"
        );
    }
}
