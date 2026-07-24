use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::{
    check::{CheckOutcome, VALID_SUFFIX, check, error_chain},
    completions::{CompletionShell, registration_line},
    report::{Report, Row, RowKind, summary_line},
};
use crate::{
    adapter::{
        clipboard,
        hooks::{HookState, HookStatus},
        path::absolutize,
    },
    domain::port::{AgentSessionStore, ProjectRegistry},
};

/// Title of the doctor report box.
const DOCTOR_TITLE: &str = "muster doctor";
/// Summary word for passing probes.
const SUMMARY_OK: &str = "ok";
/// Summary words for advisory probes.
const SUMMARY_HINT: &str = "hint";
const SUMMARY_HINTS: &str = "hints";
/// Summary words for failing probes.
const SUMMARY_FAILURE: &str = "failure";
const SUMMARY_FAILURES: &str = "failures";
/// Probe labels.
const CONFIG_LABEL: &str = "config";
const REGISTRY_LABEL: &str = "projects";
const SESSIONS_LABEL: &str = "sessions";
const HOOKS_LABEL: &str = "agent hooks";
const CLIPBOARD_LABEL: &str = "clipboard";
const COMPLETIONS_LABEL: &str = "completions";
/// Hint shown when providers need (re)installation.
const HOOKS_HINT: &str = "run `muster hooks setup`";
/// Bash shell rc file for sourcing completions.
const BASH_RC: &str = ".bashrc";
/// Zsh shell rc file for sourcing completions.
const ZSH_RC: &str = ".zshrc";
/// Fish shell rc file for sourcing completions.
const FISH_RC: &str = ".config/fish/config.fish";
/// Fish shell completions file for direct drop-in.
const FISH_COMPLETIONS: &str = ".config/fish/completions/muster.fish";
/// Elvish shell rc file for sourcing completions.
const ELVISH_RC: &str = ".elvish/rc.elv";
/// PowerShell profile for sourcing completions.
const POWERSHELL_RC: &str = ".config/powershell/Microsoft.PowerShell_profile.ps1";

/// Severity of a probe result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// Healthy.
    Ok,
    /// Advisory; does not fail the doctor run.
    Warn,
    /// Broken; the doctor run exits non-zero.
    Fail,
}

/// One diagnostic line: what was probed, how it went, and the detail text.
#[derive(Debug, Getters, CopyGetters, TypedBuilder)]
pub struct Probe {
    /// What was probed.
    #[getset(get = "pub")]
    label: String,
    /// Severity of the result.
    #[getset(get_copy = "pub")]
    outcome: ProbeOutcome,
    /// Human detail for the line.
    #[getset(get = "pub")]
    detail: String,
}

/// Builds one probe.
fn probe(label: &str, outcome: ProbeOutcome, detail: String) -> Probe {
    Probe::builder()
        .label(label.to_string())
        .outcome(outcome)
        .detail(detail)
        .build()
}

/// Validates the workspace config.
pub fn config_probe(config_path: PathBuf) -> Probe {
    let display = config_path.display().to_string();
    match check(config_path) {
        CheckOutcome::Valid => probe(
            CONFIG_LABEL,
            ProbeOutcome::Ok,
            format!("{display} {VALID_SUFFIX}"),
        ),
        CheckOutcome::Invalid(report) => probe(CONFIG_LABEL, ProbeOutcome::Fail, report),
    }
}

/// Reads the registry and flags projects whose config file is gone.
pub fn registry_probe(registry: &dyn ProjectRegistry) -> Probe {
    match registry.projects() {
        Ok(projects) => {
            let dangling: Vec<String> = projects
                .iter()
                .filter(|project| std::fs::symlink_metadata(absolutize(project.config())).is_err())
                .map(|project| project.name().as_ref().to_string())
                .collect();
            if dangling.is_empty() {
                probe(
                    REGISTRY_LABEL,
                    ProbeOutcome::Ok,
                    format!("{} registered", projects.len()),
                )
            } else {
                probe(
                    REGISTRY_LABEL,
                    ProbeOutcome::Fail,
                    format!("missing config for: {}", dangling.join(", ")),
                )
            }
        },
        Err(error) => probe(REGISTRY_LABEL, ProbeOutcome::Fail, error_chain(&error)),
    }
}

/// Confirms the agent-session store is readable.
pub fn sessions_probe(store: &dyn AgentSessionStore) -> Probe {
    match store.sessions() {
        Ok(sessions) => probe(
            SESSIONS_LABEL,
            ProbeOutcome::Ok,
            format!("{} stored", sessions.len()),
        ),
        Err(error) => probe(SESSIONS_LABEL, ProbeOutcome::Fail, error_chain(&error)),
    }
}

/// Aggregates provider hook states into one line.
pub fn hooks_probe(statuses: &[HookStatus]) -> Probe {
    let broken: Vec<String> = statuses
        .iter()
        .filter(|status| status.state() != HookState::Installed)
        .map(|status| format!("{} ({})", status.provider(), status.state()))
        .collect();
    if broken.is_empty() {
        probe(
            HOOKS_LABEL,
            ProbeOutcome::Ok,
            format!("{} providers installed", statuses.len()),
        )
    } else {
        probe(
            HOOKS_LABEL,
            ProbeOutcome::Fail,
            format!("{}; {HOOKS_HINT}", broken.join(", ")),
        )
    }
}

/// A hooks probe for when the status scan itself failed.
pub fn hooks_probe_error(error: &dyn Error) -> Probe {
    probe(HOOKS_LABEL, ProbeOutcome::Fail, error_chain(error))
}

/// Reports which clipboard path a copy would take. Informational only.
pub fn clipboard_probe() -> Probe {
    let tool = clipboard::preferred_tool();
    let detail = match (clipboard::prefers_osc52(), &tool) {
        (true, _) => "remote session; OSC 52 via the terminal".to_string(),
        (false, Some(tool_name)) => format!("native tool: {tool_name}"),
        (false, None) => "no native tool; OSC 52 via the terminal".to_string(),
    };
    let outcome = if tool.is_some() || clipboard::prefers_osc52() {
        ProbeOutcome::Ok
    } else {
        ProbeOutcome::Warn
    };
    probe(CLIPBOARD_LABEL, outcome, detail)
}

/// Best-effort check that the shell's rc file registers completions; warns
/// with the exact line to add when it does not.
pub fn completions_probe(shell_path: Option<&str>, home: &Path) -> Probe {
    let Some(shell) = shell_path.and_then(shell_from_path) else {
        return probe(
            COMPLETIONS_LABEL,
            ProbeOutcome::Warn,
            "unknown shell; see `muster completions --help`".to_string(),
        );
    };
    let matched = rc_files(shell).iter().find_map(|rc| {
        let path = home.join(rc);
        fs::read_to_string(&path)
            .ok()
            .filter(|content| content.contains(registration_line(shell)))
            .map(|_| path)
    });
    if let Some(matched) = matched {
        probe(
            COMPLETIONS_LABEL,
            ProbeOutcome::Ok,
            format!("registered in {}", matched.display()),
        )
    } else {
        probe(
            COMPLETIONS_LABEL,
            ProbeOutcome::Warn,
            format!("not registered; add: {}", registration_line(shell)),
        )
    }
}

/// The completion shell inferred from a `$SHELL` path.
fn shell_from_path(shell_path: &str) -> Option<CompletionShell> {
    let name = Path::new(shell_path).file_name()?.to_str()?;
    match name {
        "bash" => Some(CompletionShell::Bash),
        "zsh" => Some(CompletionShell::Zsh),
        "fish" => Some(CompletionShell::Fish),
        "elvish" => Some(CompletionShell::Elvish),
        "pwsh" | "powershell" => Some(CompletionShell::Powershell),
        _ => None,
    }
}

/// The rc files probed for each shell, relative to home. Fish returns two
/// locations so both the sourcing approach and the completions drop-in are
/// detected.
fn rc_files(shell: CompletionShell) -> &'static [&'static str] {
    match shell {
        CompletionShell::Bash => &[BASH_RC],
        CompletionShell::Zsh => &[ZSH_RC],
        CompletionShell::Fish => &[FISH_RC, FISH_COMPLETIONS],
        CompletionShell::Elvish => &[ELVISH_RC],
        CompletionShell::Powershell => &[POWERSHELL_RC],
    }
}

/// The report over all probes: one labeled row each, closed by a summary
/// counting outcomes (shown in the boxed layout only).
pub fn doctor_report(probes: &[Probe]) -> Report {
    let rows = probes.iter().map(probe_row).collect();
    let ok = outcome_count(probes, ProbeOutcome::Ok);
    let hints = outcome_count(probes, ProbeOutcome::Warn);
    let failures = outcome_count(probes, ProbeOutcome::Fail);
    let mut parts = vec![format!("{ok} {SUMMARY_OK}")];
    if hints > 0 {
        parts.push(format!(
            "{hints} {}",
            plural(hints, SUMMARY_HINT, SUMMARY_HINTS)
        ));
    }
    if failures > 0 {
        parts.push(format!(
            "{failures} {}",
            plural(failures, SUMMARY_FAILURE, SUMMARY_FAILURES)
        ));
    }
    Report::new(DOCTOR_TITLE, rows).with_summary(summary_line(&parts))
}

/// One probe as a report row: outcome glyph, bold label, detail.
fn probe_row(probe: &Probe) -> Row {
    let kind = match probe.outcome() {
        ProbeOutcome::Ok => RowKind::Ok,
        ProbeOutcome::Warn => RowKind::Hint,
        ProbeOutcome::Fail => RowKind::Fail,
    };
    Row::labeled(kind, probe.label().clone(), probe.detail().clone())
}

/// How many probes ended with `outcome`.
fn outcome_count(probes: &[Probe], outcome: ProbeOutcome) -> usize {
    probes
        .iter()
        .filter(|probe| probe.outcome() == outcome)
        .count()
}

/// The singular or plural word for a count.
fn plural<'a>(count: usize, singular: &'a str, many: &'a str) -> &'a str {
    if count == 1 { singular } else { many }
}

/// Whether any probe failed (warnings do not fail the run).
pub fn any_failed(probes: &[Probe]) -> bool {
    probes
        .iter()
        .any(|probe| probe.outcome() == ProbeOutcome::Fail)
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, path::Path};

    use super::*;
    use crate::domain::{
        config::ConfigError, process::AgentTool, project::Project, value::ProjectName,
    };

    /// A registry recording saves of projects and workspaces.
    #[derive(Default)]
    struct RecordingRegistry {
        projects: Vec<Project>,
        saved_projects: RefCell<Option<Vec<Project>>>,
    }

    impl crate::domain::port::ProjectRegistry for RecordingRegistry {
        fn projects(&self) -> Result<Vec<Project>, ConfigError> {
            Ok(self.projects.clone())
        }

        fn workspace(
            &self,
            _config_path: &Path,
        ) -> Result<crate::domain::config::WorkspaceConfig, ConfigError> {
            unreachable!("doctor never loads a workspace")
        }

        fn workspace_exists(&self, _config_path: &Path) -> bool {
            false
        }

        fn save(&self, projects: &[Project]) -> Result<(), ConfigError> {
            *self.saved_projects.borrow_mut() = Some(projects.to_vec());
            Ok(())
        }

        fn save_workspace(
            &self,
            _config_path: &Path,
            _config: &crate::domain::config::WorkspaceConfig,
        ) -> Result<(), ConfigError> {
            unreachable!("doctor never saves a workspace")
        }
    }

    fn project(name: &str, config: &str) -> Project {
        Project::builder()
            .name(ProjectName::try_new(name).unwrap())
            .config(PathBuf::from(config))
            .build()
    }

    fn hook_status(provider: AgentTool, state: HookState) -> HookStatus {
        HookStatus::builder()
            .provider(provider)
            .path(PathBuf::from("/dummy/path"))
            .state(state)
            .build()
    }

    /// A missing config fails the config probe.
    #[test]
    fn config_probe_fails_on_a_missing_file() {
        let probe = config_probe(std::path::PathBuf::from("/definitely/missing/muster.yml"));
        assert_eq!(probe.outcome(), ProbeOutcome::Fail);
    }

    /// Registry entries whose config is gone are flagged.
    #[test]
    fn registry_probe_flags_dangling_projects() {
        let registry = RecordingRegistry {
            projects: vec![project("gone", "/definitely/missing/muster.yml")],
            ..RecordingRegistry::default()
        };
        let probe = registry_probe(&registry);
        assert_eq!(probe.outcome(), ProbeOutcome::Fail);
        assert!(probe.detail().contains("gone"));
    }

    /// Hook statuses aggregate: any missing or stale provider fails the probe.
    #[test]
    fn hooks_probe_fails_when_any_provider_is_missing() {
        let statuses = vec![
            hook_status(AgentTool::Claude, HookState::Installed),
            hook_status(AgentTool::Codex, HookState::Missing),
        ];
        let probe = hooks_probe(&statuses);
        assert_eq!(probe.outcome(), ProbeOutcome::Fail);
        assert!(probe.detail().contains("Codex"));
    }

    /// All-installed hooks pass.
    #[test]
    fn hooks_probe_passes_when_everything_is_installed() {
        let statuses = vec![hook_status(AgentTool::Claude, HookState::Installed)];
        assert_eq!(hooks_probe(&statuses).outcome(), ProbeOutcome::Ok);
    }

    /// The clipboard probe never fails; it informs.
    #[test]
    fn clipboard_probe_is_informational() {
        let probe = clipboard_probe();
        assert_ne!(probe.outcome(), ProbeOutcome::Fail);
    }

    /// The completions probe warns with the exact line when unregistered.
    #[test]
    fn completions_probe_warns_with_the_hook_line() {
        let dir = std::env::temp_dir().join(format!("muster-doc-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".zshrc"), "# nothing here\n").unwrap();
        let probe = completions_probe(Some("/bin/zsh"), &dir);
        assert_eq!(probe.outcome(), ProbeOutcome::Warn);
        assert!(probe.detail().contains("COMPLETE=zsh"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// An rc file that mentions COMPLETE and muster in unrelated contexts must
    /// NOT be reported as registered.
    #[test]
    fn completions_probe_ignores_unrelated_complete_mention() {
        let dir = std::env::temp_dir().join(format!("muster-doc-needle-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        // Contains both words but not the actual hook line.
        std::fs::write(dir.join(".zshrc"), "# COMPLETE list for muster tasks\n").unwrap();
        let probe = completions_probe(Some("/bin/zsh"), &dir);
        assert_eq!(probe.outcome(), ProbeOutcome::Warn);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// When the hook line is in the second fish location (completions file),
    /// the probe reports Ok and names that file.
    #[test]
    fn completions_probe_detects_fish_completions_file() {
        let dir = std::env::temp_dir().join(format!("muster-doc-fish-{}", uuid::Uuid::new_v4()));
        let completions_path = dir.join(FISH_COMPLETIONS);
        std::fs::create_dir_all(completions_path.parent().unwrap()).unwrap();
        // Write only to the completions drop-in, not config.fish.
        std::fs::write(&completions_path, registration_line(CompletionShell::Fish)).unwrap();
        let probe = completions_probe(Some("/usr/bin/fish"), &dir);
        assert_eq!(probe.outcome(), ProbeOutcome::Ok);
        assert!(probe.detail().contains("completions/muster.fish"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// The completions probe detects a PowerShell profile containing the exact
    /// registration line, including the spaces around `=` that would foil a
    /// `COMPLETE=` needle.
    #[test]
    fn completions_probe_detects_powershell_registration() {
        let dir = std::env::temp_dir().join(format!("muster-pwsh-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let rc_path = dir.join(POWERSHELL_RC);
        std::fs::create_dir_all(rc_path.parent().unwrap()).unwrap();
        std::fs::write(&rc_path, registration_line(CompletionShell::Powershell)).unwrap();

        let probe = completions_probe(Some("/usr/bin/pwsh"), &dir);

        assert_eq!(probe.outcome(), ProbeOutcome::Ok);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// PowerShell paths map to the powershell completion shell.
    #[test]
    fn shell_from_path_recognizes_powershell() {
        assert_eq!(
            shell_from_path("/usr/bin/pwsh"),
            Some(CompletionShell::Powershell)
        );
    }

    /// The doctor report maps probes to labeled rows and counts the summary.
    #[test]
    fn doctor_report_maps_probes_and_summarizes() {
        let probes = vec![
            Probe::builder()
                .label("config".to_string())
                .outcome(ProbeOutcome::Ok)
                .detail("fine".to_string())
                .build(),
            Probe::builder()
                .label("completions".to_string())
                .outcome(ProbeOutcome::Warn)
                .detail("not registered".to_string())
                .build(),
        ];

        let report = doctor_report(&probes);

        assert_eq!(report.rows().len(), 2);
        assert_eq!(report.rows()[0].kind(), RowKind::Ok);
        assert_eq!(report.rows()[0].label().as_deref(), Some("config"));
        assert_eq!(report.rows()[1].kind(), RowKind::Hint);
        assert_eq!(report.summary().as_deref(), Some("1 ok · 1 hint"));
    }
}
