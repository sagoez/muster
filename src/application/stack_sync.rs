use std::collections::{HashMap, HashSet};

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::{
    application::Workspace,
    domain::{config::ProcessSpec, process::Process, value::PaneId},
};

/// Brings a live workspace in line with a project's stored stack, matching
/// processes by name key since names are unique and never renamed. Pure: the
/// supervisor acts on the panes it reports.
#[derive(Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct StackSync {
    /// The synced workspace: the stack's processes in stack order, then the
    /// retiring ones.
    workspace: Workspace,
    /// New panes, waiting in `Pending`.
    added: Vec<PaneId>,
    /// Live panes no longer in the stack, kept until they exit.
    retiring: Vec<PaneId>,
    /// Panes no longer in the stack and not live, already dropped.
    removed: Vec<PaneId>,
}

impl StackSync {
    /// Syncs `workspace` to `processes`. A name in both keeps its pane and
    /// state and takes the new definition; a new name gets the next pane id in
    /// `Pending`; a missing name is retiring when `is_live`, removed otherwise.
    pub fn apply(
        workspace: &Workspace,
        processes: &[ProcessSpec],
        is_live: impl Fn(PaneId) -> bool,
    ) -> Self {
        let mut by_key: HashMap<String, &Process> = HashMap::new();
        for process in workspace.processes() {
            by_key.entry(process.name().key()).or_insert(process);
        }
        let mut next_id = workspace
            .processes()
            .iter()
            .map(|process| process.id().into_inner() + 1)
            .max()
            .unwrap_or_default();
        let mut synced = Vec::with_capacity(processes.len());
        let mut matched = HashSet::new();
        let mut added = Vec::new();
        for spec in processes {
            match by_key.remove(&spec.name().key()) {
                Some(existing) => {
                    let mut process = existing.clone();
                    process.set_spec(spec);
                    matched.insert(*process.id());
                    synced.push(process);
                },
                None => {
                    let pane = PaneId::new(next_id);
                    next_id += 1;
                    added.push(pane);
                    synced.push(spec.to_process(pane));
                },
            }
        }
        let mut retiring = Vec::new();
        let mut removed = Vec::new();
        for process in workspace.processes() {
            let pane = *process.id();
            if matched.contains(&pane) {
                continue;
            }
            if is_live(pane) {
                retiring.push(pane);
                synced.push(process.clone());
            } else {
                removed.push(pane);
            }
        }
        Self::builder()
            .workspace(Workspace::builder().processes(synced).build())
            .added(added)
            .retiring(retiring)
            .removed(removed)
            .build()
    }

    /// Consumes the result and returns the synced workspace.
    pub fn into_workspace(self) -> Workspace {
        self.workspace
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::domain::{
        config::Environment,
        process::{ProcessState, RestartPolicy, StopPolicy, StopSignal},
        value::{CommandLine, ProcessName, StopGrace},
    };

    /// `café` with a precomposed `é`.
    const CAFE_COMPOSED: &str = "caf\u{e9}";
    /// `café` with `e` and a combining acute accent.
    const CAFE_DECOMPOSED: &str = "cafe\u{301}";

    /// A definition of `name` running `command`.
    fn spec(name: &str, command: &str) -> ProcessSpec {
        ProcessSpec::builder()
            .name(ProcessName::try_new(name).unwrap())
            .command(CommandLine::try_new(command).unwrap())
            .environment(Environment::empty())
            .autostart(true)
            .restart(RestartPolicy::Never)
            .stop(
                StopPolicy::builder()
                    .signal(StopSignal::Terminate)
                    .grace_period(StopGrace::try_new(Duration::from_secs(5)).unwrap())
                    .build(),
            )
            .build()
    }

    /// Process `name` on pane `id`, running `command` and in `state`.
    fn process(id: u64, name: &str, command: &str, state: ProcessState) -> Process {
        let mut process = spec(name, command).to_process(PaneId::new(id));
        process.set_state(state);
        process
    }

    /// web running on pane 0, api exited on pane 1, worker unsupervised on 2.
    fn workspace() -> Workspace {
        Workspace::builder()
            .processes(vec![
                process(0, "web", "npm run dev", ProcessState::Running),
                process(1, "api", "cargo run", ProcessState::Exited),
                process(2, "worker", "bin/worker", ProcessState::Unsupervised),
            ])
            .build()
    }

    /// Live as the supervisor counts it: a child may still be running.
    fn is_live(workspace: &Workspace) -> impl Fn(PaneId) -> bool + '_ {
        |pane| {
            workspace
                .process(pane)
                .is_some_and(|process| process.state().may_be_running())
        }
    }

    /// The names of `workspace`'s processes, in order.
    fn names(workspace: &Workspace) -> Vec<String> {
        workspace
            .processes()
            .iter()
            .map(|process| process.name().to_string())
            .collect()
    }

    #[test]
    fn an_unchanged_stack_keeps_every_pane_and_state() {
        let before = workspace();

        let sync = StackSync::apply(
            &before,
            &[
                spec("web", "npm run dev"),
                spec("api", "cargo run"),
                spec("worker", "bin/worker"),
            ],
            is_live(&before),
        );

        assert!(sync.added().is_empty());
        assert!(sync.retiring().is_empty());
        assert!(sync.removed().is_empty());
        let after = sync.into_workspace();
        assert_eq!(names(&after), ["web", "api", "worker"]);
        assert_eq!(
            *after.process(PaneId::new(0)).unwrap().state(),
            ProcessState::Running
        );
    }

    /// A changed definition updates the pane in place and keeps it running.
    #[test]
    fn a_changed_process_keeps_its_pane_and_state() {
        let before = workspace();

        let sync = StackSync::apply(
            &before,
            &[
                spec("web", "pnpm dev"),
                spec("api", "cargo run"),
                spec("worker", "bin/worker"),
            ],
            is_live(&before),
        );

        let web = sync.workspace().process(PaneId::new(0)).unwrap();
        assert_eq!(web.command().as_ref(), "pnpm dev");
        assert_eq!(*web.state(), ProcessState::Running);
    }

    /// A new name takes the next free pane id and waits in `Pending`.
    #[test]
    fn a_new_process_gets_the_next_pane_pending() {
        let before = workspace();

        let sync = StackSync::apply(
            &before,
            &[
                spec("web", "npm run dev"),
                spec("docs", "mdbook serve"),
                spec("api", "cargo run"),
                spec("worker", "bin/worker"),
            ],
            is_live(&before),
        );

        assert_eq!(*sync.added(), [PaneId::new(3)]);
        let docs = sync.workspace().process(PaneId::new(3)).unwrap();
        assert_eq!(*docs.state(), ProcessState::Pending);
        assert_eq!(
            names(sync.workspace()),
            ["web", "docs", "api", "worker"],
            "order follows the stack"
        );
    }

    /// Missing names retire when live, unsupervised included, and are dropped
    /// otherwise; retiring panes stay after the stack's processes.
    #[test]
    fn missing_processes_retire_when_live_and_are_removed_otherwise() {
        let before = workspace();

        let sync = StackSync::apply(&before, &[spec("docs", "mdbook serve")], is_live(&before));

        assert_eq!(*sync.retiring(), [PaneId::new(0), PaneId::new(2)]);
        assert_eq!(*sync.removed(), [PaneId::new(1)]);
        assert_eq!(*sync.added(), [PaneId::new(3)]);
        assert_eq!(names(sync.workspace()), ["docs", "web", "worker"]);
    }

    /// Names are matched by key, so a respelt accent keeps the pane.
    #[test]
    fn names_match_by_nfc_key() {
        let before = Workspace::builder()
            .processes(vec![process(
                0,
                CAFE_COMPOSED,
                "true",
                ProcessState::Running,
            )])
            .build();

        let sync = StackSync::apply(&before, &[spec(CAFE_DECOMPOSED, "true")], is_live(&before));

        assert!(sync.added().is_empty());
        assert_eq!(
            sync.workspace().processes()[0].name().as_ref(),
            CAFE_DECOMPOSED
        );
    }

    #[test]
    fn an_empty_workspace_adds_from_pane_zero() {
        let before = Workspace::builder().processes(Vec::new()).build();

        let sync = StackSync::apply(&before, &[spec("web", "true")], |_| false);

        assert_eq!(*sync.added(), [PaneId::new(0)]);
    }
}
