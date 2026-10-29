use std::collections::HashMap;

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::{
    application::Workspace,
    domain::{
        config::{ProcessSpec, WorkspaceConfig},
        process::{Process, ProcessKind, ProcessOrigin, RestartPolicy, StopPolicy},
        value::{CommandLine, Description, PaneId, ProcessName},
    },
};

/// Applies a workspace configuration to the live process model without I/O.
#[derive(Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Reconciliation {
    /// Rebuilt workspace preserving every surviving process identity.
    workspace: Workspace,
    /// Configured panes that continue to be represented by disk configuration.
    tracked: Vec<PaneId>,
    /// Live panes removed from disk that must retire after their current exit.
    retiring: Vec<PaneId>,
    /// Stopped panes that should be discarded immediately.
    removed: Vec<PaneId>,
}

impl Reconciliation {
    /// Reconciles `workspace` with `config`, consulting `is_live` before
    /// removing a process whose configuration entry disappeared.
    pub fn apply(
        workspace: &Workspace,
        config: &WorkspaceConfig,
        is_live: impl Fn(PaneId) -> bool,
    ) -> Self {
        let sections = [
            (ProcessKind::Agent, config.agents()),
            (ProcessKind::Terminal, config.terminals()),
            (ProcessKind::Command, config.commands()),
        ];
        let mut config_counts = HashMap::new();
        for (kind, specs) in sections {
            for spec in specs {
                *config_counts
                    .entry(ProcessSpecIdentity::of_spec(kind, spec))
                    .or_insert(0_usize) += 1;
            }
        }

        let mut kept = Vec::new();
        let mut tracked = Vec::new();
        let mut retiring = Vec::new();
        let mut removed = Vec::new();
        let mut next_id = 0;
        for process in workspace.processes() {
            next_id = next_id.max((*process.id()).into_inner() + 1);
            if *process.origin() == ProcessOrigin::Session {
                kept.push(process.clone());
                continue;
            }
            let identity = ProcessSpecIdentity::of_process(process);
            let pane = *process.id();
            let matched = config_counts
                .get_mut(&identity)
                .filter(|count| **count > 0)
                .map(|count| *count -= 1)
                .is_some();
            if matched {
                tracked.push(pane);
                kept.push(process.clone());
            } else if is_live(pane) {
                retiring.push(pane);
                kept.push(process.clone());
            } else {
                removed.push(pane);
            }
        }

        for (kind, specs) in sections {
            for spec in specs {
                if let Some(count) = config_counts
                    .get_mut(&ProcessSpecIdentity::of_spec(kind, spec))
                    .filter(|count| **count > 0)
                {
                    *count -= 1;
                    kept.push(spec.to_process(PaneId::new(next_id), kind));
                    next_id += 1;
                }
            }
        }

        kept.sort_by_key(|process| process.kind().section_index());
        Self::builder()
            .workspace(Workspace::builder().processes(kept).build())
            .tracked(tracked)
            .retiring(retiring)
            .removed(removed)
            .build()
    }

    /// Consumes the result and returns the reconciled workspace.
    pub fn into_workspace(self) -> Workspace {
        self.workspace
    }
}

/// Full resolved identity of one configured process.
#[derive(Hash, PartialEq, Eq)]
struct ProcessSpecIdentity {
    kind: ProcessKind,
    name: ProcessName,
    command: Option<CommandLine>,
    working_dir: Option<std::path::PathBuf>,
    description: Option<Description>,
    restart: RestartPolicy,
    stop: Option<StopPolicy>,
    autostart: bool,
}

impl ProcessSpecIdentity {
    /// Builds the resolved identity of one configuration entry.
    fn of_spec(kind: ProcessKind, spec: &ProcessSpec) -> Self {
        Self {
            kind,
            name: spec.name().clone(),
            command: spec.command().clone(),
            working_dir: spec.working_dir().clone(),
            description: spec.description().clone(),
            restart: spec.restart_policy(),
            stop: spec.effective_stop_policy(kind),
            autostart: *spec.autostart(),
        }
    }

    /// Builds the resolved identity of one live configured process.
    fn of_process(process: &Process) -> Self {
        Self {
            kind: *process.kind(),
            name: process.name().clone(),
            command: process.command().clone(),
            working_dir: process.working_dir().clone(),
            description: process.description().clone(),
            restart: *process.restart(),
            stop: process.effective_stop_policy(),
            autostart: *process.autostart(),
        }
    }
}
