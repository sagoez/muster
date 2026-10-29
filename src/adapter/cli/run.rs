//! `muster run`: a command added to a project, started under the service and
//! attached to.

use getset::Getters;
use typed_builder::TypedBuilder;

use super::{
    command_words::command_string,
    error::{CliError, RUN_NAME_HINT},
    invocation::Invocation,
    run_args::RunArgs,
    working_dir::stored_working_dir,
};
use crate::{
    adapter::client::{
        ActionReport, ProjectTarget, ServiceClient, SettleWatch,
        process::{ready_to_send, start_added},
    },
    application::StackEditor,
    domain::{
        config::ProcessDraft,
        coordination::Author,
        port::LocalState,
        project::Project,
        value::{ProcessName, ProjectName},
    },
};

/// Whether `muster up` starts a process `muster run` adds. Off: running a
/// command once does not enlist it in every later start of the stack.
const RUN_AUTOSTART: bool = false;

/// A `muster run` checked and ready: the project to add to, and the process
/// to add. Nothing is written yet.
#[derive(Debug, Clone, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct RunPlan {
    /// The project, registered or not.
    target: ProjectTarget,
    /// The process to add.
    draft: ProcessDraft,
}

impl RunPlan {
    /// Checks `args` and finds the project to add the command to, which
    /// works in the folder it is typed in. A name the project already has
    /// is refused here, before the service is reached.
    ///
    /// # Errors
    /// Returns [`CliError`] when the command or name is not usable, the name
    /// is taken, or the project cannot be resolved.
    pub fn prepare(
        invocation: &Invocation<'_>,
        store: &dyn LocalState,
        args: &RunArgs,
    ) -> Result<Self, CliError> {
        let command = command_string(&args.command)?;
        let name = process_name(args.name.as_deref(), &args.command)?;
        let target = invocation.target(store)?;
        if let ProjectTarget::Registered(project) = &target
            && store
                .stack(project.root())?
                .is_some_and(|stack| stack.process(&name).is_some())
        {
            return Err(CliError::ProcessNameTaken {
                name,
                project: project.name().clone(),
                hint: RUN_NAME_HINT,
            });
        }
        let working_dir = stored_working_dir(target.root(), &invocation.cwd()?)?;
        let draft = ProcessDraft::builder()
            .name(name)
            .command(command)
            .working_dir(working_dir)
            .autostart(Some(RUN_AUTOSTART))
            .build();
        Ok(Self::builder().target(target).draft(draft).build())
    }

    /// The project's name, or the one the folder would be registered as.
    pub fn project_name(&self) -> Option<ProjectName> {
        self.target.name()
    }

    /// Adds the command to its project, registering the folder when new,
    /// then starts it through `client` and reports it once it settled,
    /// telling `watch` meanwhile.
    ///
    /// # Errors
    /// Returns [`CliError::ProcessNameTaken`] when a concurrent writer took
    /// the name, or another [`CliError`] when the store refuses or the
    /// service cannot start the command.
    pub fn carry_out(
        self,
        editor: &StackEditor<'_>,
        client: &ServiceClient,
        watch: &mut dyn SettleWatch,
    ) -> Result<(Project, ActionReport), CliError> {
        ready_to_send(watch)?;
        let name = self.draft.name().clone();
        let (registration, _) = editor
            .add(self.target.root(), self.draft, &Author::human())
            .map_err(|error| CliError::adding(error, self.target.name(), RUN_NAME_HINT))?;
        let project = registration.project().clone();
        let report = start_added(client, &project, name, watch)?;
        Ok((project, report))
    }
}

/// The process name to record: an explicit non-blank name, else the
/// command's first word.
///
/// # Errors
/// Returns [`CliError::InvalidName`] when neither gives one.
fn process_name(explicit: Option<&str>, command: &[String]) -> Result<ProcessName, CliError> {
    let candidate = explicit
        .map(str::trim)
        .filter(|value| !value.is_empty())
        // The first word of the first argument: for a single quoted expression
        // like `npm test && npm run build` this is `npm`, not the whole string.
        .or_else(|| {
            command
                .first()
                .and_then(|first| first.split_whitespace().next())
        })
        .unwrap_or_default();
    ProcessName::try_new(candidate).map_err(|error| CliError::invalid_name(candidate, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::value::ProcessNameFault;

    #[test]
    fn the_process_name_defaults_to_the_first_word() {
        let command = vec!["npm".to_string(), "run".to_string(), "dev".to_string()];
        assert_eq!(process_name(None, &command).unwrap().as_ref(), "npm");
        assert_eq!(process_name(Some("web"), &command).unwrap().as_ref(), "web");
        assert_eq!(
            process_name(Some("   "), &command).unwrap().as_ref(),
            "npm",
            "a blank explicit name falls back to the first word"
        );
        assert_eq!(
            process_name(None, &["npm test && npm run build".to_string()])
                .unwrap()
                .as_ref(),
            "npm",
            "a single quoted expression uses its first word, not the whole string"
        );
        assert!(matches!(
            process_name(None, &["  ".to_string()]),
            Err(CliError::InvalidName {
                fault: ProcessNameFault::Empty,
                ..
            })
        ));
    }

    /// A name holding an escape sequence is refused for that reason, and the
    /// message shows the sequence as text.
    #[test]
    fn a_control_character_name_is_explained_and_escaped() {
        let refused = process_name(Some("web\u{1b}]0;x\u{7}"), &[]).unwrap_err();

        assert!(matches!(refused, CliError::InvalidName {
            fault: ProcessNameFault::ControlCharacter,
            ..
        }));
        let message = refused.to_string();
        assert!(!message.chars().any(char::is_control), "{message}");
        assert!(message.contains("web\\u{1b}]0;x\\u{7}"), "{message}");
        assert!(message.contains("control characters"), "{message}");
    }
}
