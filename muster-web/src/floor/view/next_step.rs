use getset::{CopyGetters, Getters};
use muster_domain::{
    process::ProcessState,
    value::{ProcessName, ProjectName, process_operand, project_option},
};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::{next_verb::NextVerb, placement::Placement};

/// The program every suggested command runs.
const PROGRAM: &str = "muster";

/// The command a card suggests for one process: what it does, and the whole
/// line to paste, naming the project so it works from any folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub(crate) struct NextStep {
    /// What it does.
    #[getset(get_copy = "pub")]
    verb: NextVerb,
    /// The line to paste.
    #[getset(get = "pub")]
    command: String,
}

impl NextStep {
    /// The step for process `name` of `project`, standing at `placement` in
    /// `state`; none for a name no shell word can carry.
    #[must_use]
    pub(crate) fn of(
        project: &ProjectName,
        name: &ProcessName,
        placement: Placement,
        state: ProcessState,
    ) -> Option<Self> {
        let verb = NextVerb::of(placement, state);
        let word: &'static str = verb.into();
        let mut command = format!("{PROGRAM} {} {word}", project_option(project)?);
        if verb.names_the_process() {
            command = format!("{command} {}", process_operand(name)?);
        }
        Some(Self::builder().verb(verb).command(command).build())
    }
}

#[cfg(test)]
mod tests {
    use muster_domain::process::Readiness;

    use super::*;
    use crate::floor::fixture::process_status;

    /// The step for `name` of `project` in `state`.
    fn step(project: &str, name: &str, state: ProcessState) -> NextStep {
        let status = process_status(name, state, Readiness::None);
        NextStep::of(
            &ProjectName::try_new(project).unwrap(),
            &ProcessName::try_new(name).unwrap(),
            Placement::of(Some(&status)),
            state,
        )
        .unwrap()
    }

    /// Each state gets the command that acts on it: a crash its logs, a
    /// blocked run the notices, a pause a resume, an unsupervised child a
    /// stop, an exit a start, and a running process an attach.
    #[test]
    fn each_state_suggests_the_command_that_acts_on_it() {
        let cases = [
            (
                ProcessState::Crashed,
                NextVerb::Logs,
                "muster -p shop logs web",
            ),
            (
                ProcessState::Blocked,
                NextVerb::Notices,
                "muster -p shop notices",
            ),
            (
                ProcessState::Paused,
                NextVerb::Resume,
                "muster -p shop resume web",
            ),
            (
                ProcessState::Unsupervised,
                NextVerb::Stop,
                "muster -p shop stop web",
            ),
            (
                ProcessState::Exited,
                NextVerb::Start,
                "muster -p shop start web",
            ),
            (
                ProcessState::Running,
                NextVerb::Attach,
                "muster -p shop attach web",
            ),
            (
                ProcessState::Restarting,
                NextVerb::Attach,
                "muster -p shop attach web",
            ),
        ];
        for (state, verb, command) in cases {
            let step = step("shop", "web", state);
            assert_eq!(
                (step.verb(), step.command().as_str()),
                (verb, command),
                "{state}"
            );
        }
    }

    /// A project or a process whose name reads as a flag is passed so the
    /// command line takes it as a name.
    #[test]
    fn a_name_that_reads_as_a_flag_stays_a_name() {
        assert_eq!(
            step("-shop", "--web", ProcessState::Crashed).command(),
            "muster --project=-shop logs -- --web"
        );
        assert_eq!(
            step("my shop", "dev server", ProcessState::Exited).command(),
            "muster -p 'my shop' start 'dev server'"
        );
    }
}
