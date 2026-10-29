use std::path::Path;

use super::{config_membership::ConfigMembership, entity::Supervisor};
use crate::{adapter::path, application::Reconciliation, domain::config::WorkspaceConfig};

impl Supervisor {
    /// Points the config watcher at the project's config.
    pub(super) fn rewatch_config(&mut self) {
        if let Some(watcher) = self.config_watcher.as_mut() {
            watcher.watch(self.project.as_ref());
        }
    }

    /// Reconciles the workspace when the config at `path` changed, if it is
    /// this project's. A config that cannot be read, as mid-write, is ignored
    /// until its next change.
    pub fn handle_config_changed(&mut self, path: &Path) {
        if path::normalize(self.project.as_ref()) != path::normalize(path) {
            return;
        }
        let Ok(config) = self.registry.workspace(self.project.as_ref()) else {
            return;
        };
        self.reconcile_config(&config);
    }

    /// Applies a changed config without bouncing surviving processes: a
    /// removed live process retires on exit, a removed stopped one goes now,
    /// and configured agents are re-linked to their durable sessions.
    pub(super) fn reconcile_config(&mut self, config: &WorkspaceConfig) {
        let reconciliation =
            Reconciliation::apply(&self.workspace, config, |pane| self.holds_process(pane));
        for pane in reconciliation.tracked().clone() {
            self.set_membership(pane, ConfigMembership::Tracked);
        }
        for pane in reconciliation.retiring().clone() {
            self.set_membership(pane, ConfigMembership::RetireOnExit);
        }
        for pane in reconciliation.removed().clone() {
            // Retiring advances the spawn generation, so a queued `ForceStop` or
            // `Respawn` cannot match a process that reuses this pane id.
            self.retire_pane(pane);
        }
        self.workspace = reconciliation.into_workspace();
        // Reconcile rebuilds processes with no session ids and does not spawn,
        // so a link failure here is retried on the next start.
        self.link_configured_agent_sessions();
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use super::*;
    use crate::{
        adapter::service::supervisor::testing::*,
        domain::{
            config::ProcessSpec,
            port::ProcessRunner,
            process::{
                AgentTool, Process, ProcessKind, ProcessOrigin, ProcessState, RestartPolicy,
                StopPolicy, StopSignal,
            },
            pty::{ExitOutcome, ProcessOutput},
            value::{CommandLine, PaneId, ProcessName},
        },
    };

    const PANE: u64 = 0;
    /// Grace period of the graceful test stop policy.
    const TEST_STOP_GRACE: Duration = Duration::from_secs(7);

    /// `name` as a process name.
    fn name(name: &str) -> ProcessName {
        ProcessName::try_new(name).unwrap()
    }

    /// A process spec named `spec_name` running `command`, not autostarting,
    /// as a command usually is.
    fn spec(spec_name: &str, command: &str) -> ProcessSpec {
        ProcessSpec::builder()
            .name(name(spec_name))
            .command(Some(CommandLine::try_new(command).unwrap()))
            .autostart(false)
            .build()
    }

    /// As [`spec`], autostarting, as a terminal or agent usually is.
    fn autostarting(spec_name: &str, command: &str) -> ProcessSpec {
        spec(spec_name, command).with_autostart(true)
    }

    /// An autostarting `db` command with a graceful interrupt stop policy.
    fn graceful_db() -> ProcessSpec {
        ProcessSpec::builder()
            .name(name("db"))
            .command(Some(CommandLine::try_new("postgres").unwrap()))
            .autostart(true)
            .stop(Some(
                StopPolicy::builder()
                    .signal(StopSignal::Interrupt)
                    .grace_period(TEST_STOP_GRACE)
                    .build(),
            ))
            .build()
    }

    /// A supervisor of `processes` spawning through `runner`, whose registry
    /// reads `config` and records each write.
    fn over_config(
        runner: Box<dyn ProcessRunner>,
        processes: Vec<Process>,
        config: WorkspaceConfig,
    ) -> Harness {
        Harness::with_registry(
            runner,
            processes,
            Box::new(FakeRegistry {
                projects: Vec::new(),
                workspace: config,
                recorder: Recorder::default(),
            }),
        )
    }

    /// A supervisor over `loaded`'s processes whose config now reads `updated`.
    fn loaded_then(loaded: &WorkspaceConfig, updated: WorkspaceConfig) -> Harness {
        over_config(Box::new(FakeRunner), loaded.to_processes(), updated)
    }

    /// A started supervisor running one terminal `p`, whose config now reads
    /// `config`.
    fn running_p(config: WorkspaceConfig) -> Harness {
        let p = Process::builder()
            .id(PaneId::new(PANE))
            .name(name("p"))
            .kind(ProcessKind::Terminal)
            .command(Some(CommandLine::try_new("true").unwrap()))
            .restart(RestartPolicy::Never)
            .build();
        let mut harness = over_config(Box::new(FakeRunner), vec![p], config);
        harness.supervisor.up();
        harness
    }

    /// A started supervisor of `config`'s processes spawning through a runner
    /// that records stop signals.
    fn graceful(config: WorkspaceConfig) -> Harness {
        let mut harness = Harness::with_registry(
            Box::new(StopSignalRunner {
                signals: Arc::new(StopSignals::default()),
            }),
            config.to_processes(),
            Box::new(StatefulRegistry {
                recorder: Recorder::default(),
                initial: config,
            }),
        );
        harness.supervisor.up();
        harness
    }

    /// Tells the supervisor its own config changed.
    fn config_changed(harness: &mut Harness) {
        harness
            .supervisor
            .handle_config_changed(Path::new(TEST_PROJECT));
    }

    /// The names of the workspace's processes, in order.
    fn names(harness: &Harness) -> Vec<String> {
        harness
            .supervisor
            .workspace()
            .processes()
            .iter()
            .map(|process| process.name().as_ref().to_string())
            .collect()
    }

    /// The command lines of the workspace's processes, in order.
    fn commands(harness: &Harness) -> Vec<String> {
        harness
            .supervisor
            .workspace()
            .processes()
            .iter()
            .filter_map(|process| process.command().as_ref().map(|c| c.as_ref().to_string()))
            .collect()
    }

    /// The state of the first process named `process`.
    fn state_named(harness: &Harness, process: &str) -> ProcessState {
        *harness
            .supervisor
            .workspace()
            .processes()
            .iter()
            .find(|candidate| candidate.name().as_ref() == process)
            .unwrap()
            .state()
    }

    /// Whether `pane` still belongs to the config.
    fn membership(harness: &Harness, pane: PaneId) -> ConfigMembership {
        harness.supervisor.panes()[&pane].config_membership
    }

    /// Delivers `outcome` as `pane`'s exit at its current generation.
    fn exit(harness: &mut Harness, pane: PaneId, outcome: ExitOutcome) {
        let generation = harness.supervisor.generations()[&pane];
        harness
            .supervisor
            .handle_output(pane, generation, ProcessOutput::Exited(outcome));
    }

    #[test]
    fn a_config_change_adds_new_processes_without_touching_running_ones() {
        let config = empty_workspace_config()
            .with_terminals(vec![autostarting("p", "true")])
            .with_commands(vec![spec("q", "echo q")]);
        let mut harness = running_p(config);

        config_changed(&mut harness);

        assert_eq!(names(&harness), vec!["p".to_string(), "q".to_string()]);
        assert_eq!(
            state_named(&harness, "p"),
            ProcessState::Running,
            "the already-running process is left untouched"
        );
        assert_eq!(
            state_named(&harness, "q"),
            ProcessState::Pending,
            "the new process appears stopped, not auto-started"
        );
    }

    #[test]
    fn reconciliation_matches_by_full_spec_not_just_name() {
        let loaded = empty_workspace_config().with_commands(vec![spec("npm", "npm test")]);
        let updated = empty_workspace_config()
            .with_commands(vec![spec("npm", "npm run dev"), spec("npm", "npm test")]);
        let mut harness = loaded_then(&loaded, updated);

        config_changed(&mut harness);

        assert_eq!(
            commands(&harness),
            vec!["npm test".to_string(), "npm run dev".to_string()],
            "the differently-commanded same-named spec is added; the matching one is not duplicated"
        );
    }

    #[test]
    fn reconciliation_removes_a_stopped_process_dropped_from_config() {
        let loaded =
            empty_workspace_config().with_commands(vec![spec("a", "cmd-a"), spec("b", "cmd-b")]);
        let updated = empty_workspace_config().with_commands(vec![spec("a", "cmd-a")]);
        let mut harness = loaded_then(&loaded, updated);

        config_changed(&mut harness);

        assert_eq!(names(&harness), vec!["a".to_string()]);
    }

    #[test]
    fn reconciliation_keeps_a_running_process_absent_from_config() {
        let mut harness = running_p(empty_workspace_config());

        config_changed(&mut harness);

        assert!(
            names(&harness).contains(&"p".to_string()),
            "a running process not in the config is preserved"
        );
    }

    #[test]
    fn reconciliation_replaces_a_stopped_process_whose_command_changed() {
        let loaded = empty_workspace_config().with_commands(vec![spec("test", "npm test")]);
        let updated = empty_workspace_config().with_commands(vec![spec("test", "npm run dev")]);
        let mut harness = loaded_then(&loaded, updated);

        config_changed(&mut harness);

        assert_eq!(commands(&harness), vec!["npm run dev".to_string()]);
    }

    /// A supervisor whose live model omits `stop` while the reloaded config
    /// writes the equivalent default policy explicitly.
    fn equivalent_stop_defaults() -> Harness {
        let implicit = spec("server", "serve");
        let explicit = implicit.clone().with_stop(Some(StopPolicy::default()));
        let loaded = empty_workspace_config().with_commands(vec![implicit]);
        let updated = empty_workspace_config().with_commands(vec![explicit]);
        loaded_then(&loaded, updated)
    }

    /// Writing the implicit stop defaults explicitly does not replace a
    /// stopped pane, since both behave the same.
    #[test]
    fn reconciliation_normalizes_equivalent_stop_defaults_for_a_stopped_command() {
        let mut harness = equivalent_stop_defaults();
        let original = *harness.supervisor.workspace().processes()[0].id();

        config_changed(&mut harness);

        assert_eq!(harness.supervisor.workspace().processes().len(), 1);
        assert_eq!(
            *harness.supervisor.workspace().processes()[0].id(),
            original
        );
    }

    /// Writing the implicit stop defaults explicitly neither retires nor
    /// duplicates a running command.
    #[test]
    fn reconciliation_normalizes_equivalent_stop_defaults_for_a_running_command() {
        let mut harness = equivalent_stop_defaults();
        let pane = *harness.supervisor.workspace().processes()[0].id();
        harness.supervisor.start(&name("server"));

        config_changed(&mut harness);

        assert_eq!(harness.supervisor.workspace().processes().len(), 1);
        assert_eq!(membership(&harness, pane), ConfigMembership::Tracked);
    }

    #[test]
    fn a_live_process_removed_from_config_is_retired_when_it_exits() {
        let mut harness = running_p(empty_workspace_config());
        let pane = PaneId::new(PANE);

        config_changed(&mut harness);
        assert!(
            harness.supervisor.workspace().process(pane).is_some(),
            "a running process is kept while alive"
        );

        exit(&mut harness, pane, ExitOutcome::Succeeded);
        assert!(
            harness.supervisor.workspace().process(pane).is_none(),
            "the orphaned process is removed once it exits"
        );
    }

    #[test]
    fn reconciliation_reflects_a_restart_policy_change() {
        let with_restart = |restart| {
            ProcessSpec::builder()
                .name(name("w"))
                .command(Some(CommandLine::try_new("run").unwrap()))
                .restart(Some(restart))
                .autostart(false)
                .build()
        };
        let loaded =
            empty_workspace_config().with_commands(vec![with_restart(RestartPolicy::Never)]);
        let updated =
            empty_workspace_config().with_commands(vec![with_restart(RestartPolicy::Always)]);
        let mut harness = loaded_then(&loaded, updated);

        config_changed(&mut harness);

        let policies: Vec<RestartPolicy> = harness
            .supervisor
            .workspace()
            .processes()
            .iter()
            .map(|process| *process.restart())
            .collect();
        assert_eq!(policies, vec![RestartPolicy::Always]);
    }

    #[test]
    fn reconciliation_reflects_an_autostart_change() {
        let with_autostart = |autostart| {
            ProcessSpec::builder()
                .name(name("w"))
                .command(Some(CommandLine::try_new("run").unwrap()))
                .autostart(autostart)
                .build()
        };
        let loaded = empty_workspace_config().with_commands(vec![with_autostart(false)]);
        let updated = empty_workspace_config().with_commands(vec![with_autostart(true)]);
        let mut harness = loaded_then(&loaded, updated);

        config_changed(&mut harness);

        let autostarts: Vec<bool> = harness
            .supervisor
            .workspace()
            .processes()
            .iter()
            .map(|process| *process.autostart())
            .collect();
        assert_eq!(autostarts, vec![true]);
    }

    #[test]
    fn a_config_change_for_a_different_path_is_ignored() {
        let mut harness = running_p(empty_workspace_config());

        harness
            .supervisor
            .handle_config_changed(Path::new("/elsewhere/muster.yml"));

        assert_eq!(names(&harness), vec!["p".to_string()]);
    }

    /// Live config reconciliation never treats a runtime session as a spec.
    #[test]
    fn config_reconciliation_keeps_runtime_agent_sessions() {
        let mut harness = running_p(empty_workspace_config());
        let pane = harness
            .supervisor
            .create_agent_session(AgentTool::Gemini, None, None, None)
            .unwrap()
            .pane();

        config_changed(&mut harness);

        assert_eq!(
            *harness
                .supervisor
                .workspace()
                .process(pane)
                .unwrap()
                .origin(),
            ProcessOrigin::Session
        );
    }

    /// An external edit that removes a running command then restores it
    /// reclaims the running pane rather than spawning a duplicate.
    #[test]
    fn an_externally_restored_spec_reclaims_its_retiring_pane() {
        let with = empty_workspace_config().with_commands(vec![graceful_db()]);
        let mut harness = graceful(with.clone());
        let pane = PaneId::new(PANE);

        harness
            .supervisor
            .reconcile_config(&empty_workspace_config());
        assert_eq!(membership(&harness, pane), ConfigMembership::RetireOnExit);

        harness.supervisor.reconcile_config(&with);

        assert_eq!(membership(&harness, pane), ConfigMembership::Tracked);
        assert!(harness.supervisor.is_live(pane));
        assert_eq!(harness.supervisor.workspace().processes().len(), 1);
    }
}
