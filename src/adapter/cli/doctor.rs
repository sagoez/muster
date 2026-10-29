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
    report::{Report, Row, RowKind},
};
use crate::{
    adapter::{
        hooks::{HookState, HookStatus},
        path::absolutize,
    },
    domain::port::{AgentSessionStore, ProjectRegistry},
};

/// Title of the doctor report.
const DOCTOR_TITLE: &str = "muster doctor";
/// Probe labels.
const CONFIG_LABEL: &str = "config";
const REGISTRY_LABEL: &str = "projects";
const SESSIONS_LABEL: &str = "sessions";
const HOOKS_LABEL: &str = "agent hooks";
const COMPLETIONS_LABEL: &str = "completions";
/// Prefix of a commented-out rc line, ignored by every probed shell.
const COMMENT_PREFIX: &str = "#";
/// Hint shown when providers need (re)installation.
const HOOKS_HINT: &str = "run `muster hooks setup`";
/// Bash shell rc file for sourcing completions.
const BASH_RC: &str = ".bashrc";
/// Zsh shell rc file for sourcing completions.
const ZSH_RC: &str = ".zshrc";
/// Fish shell rc file for sourcing completions.
const FISH_RC: &str = "fish/config.fish";
/// Fish shell completions file for direct drop-in.
const FISH_COMPLETIONS: &str = "fish/completions/muster.fish";
/// Elvish shell rc file for sourcing completions.
const ELVISH_RC: &str = ".elvish/rc.elv";
/// Elvish rc under the XDG config root, the modern location.
const ELVISH_XDG_RC: &str = "elvish/rc.elv";
/// PowerShell profile for sourcing completions.
const POWERSHELL_RC: &str = "powershell/Microsoft.PowerShell_profile.ps1";

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
        Ok(CheckOutcome::Valid) => probe(
            CONFIG_LABEL,
            ProbeOutcome::Ok,
            format!("{display} {VALID_SUFFIX}"),
        ),
        Ok(CheckOutcome::Invalid(report)) => probe(CONFIG_LABEL, ProbeOutcome::Fail, report),
        // The doctor is a reporter: even an unreadable config becomes a
        // failing probe rather than aborting the other probes.
        Err(error) => probe(CONFIG_LABEL, ProbeOutcome::Fail, error_chain(&error)),
    }
}

/// Reads the registry and flags projects whose config file is not a reachable
/// regular file - missing, a directory, or a dangling symlink all fail.
pub fn registry_probe(registry: &dyn ProjectRegistry) -> Probe {
    match registry.projects() {
        Ok(projects) => {
            let dangling: Vec<String> = projects
                .iter()
                .filter(|project| {
                    !fs::metadata(absolutize(project.config()))
                        .map(|meta| meta.is_file())
                        .unwrap_or(false)
                })
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

/// Best-effort check that the shell's rc file registers completions; warns
/// with the exact line to add when it does not.
pub fn completions_probe(shell_path: Option<&str>, home: &Path, xdg_config: &Path) -> Probe {
    let Some(shell) = shell_path.and_then(shell_from_path) else {
        return probe(
            COMPLETIONS_LABEL,
            ProbeOutcome::Warn,
            "unknown shell; see `muster completions --help`".to_string(),
        );
    };
    let matched = rc_paths(shell, home, xdg_config).into_iter().find(|path| {
        fs::read_to_string(path).is_ok_and(|content| registers_completions(&content, shell))
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

/// Whether any active (non-comment) line contains the shell's registration
/// hook; a commented-out line is never evaluated by the shell.
fn registers_completions(content: &str, shell: CompletionShell) -> bool {
    content.lines().any(|line| {
        let active = line.trim_start();
        !active.starts_with(COMMENT_PREFIX) && active.contains(registration_line(shell))
    })
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
fn rc_paths(shell: CompletionShell, home: &Path, xdg_config: &Path) -> Vec<PathBuf> {
    match shell {
        CompletionShell::Bash => vec![home.join(BASH_RC)],
        CompletionShell::Zsh => vec![home.join(ZSH_RC)],
        CompletionShell::Fish => vec![xdg_config.join(FISH_RC), xdg_config.join(FISH_COMPLETIONS)],
        CompletionShell::Elvish => vec![home.join(ELVISH_RC), xdg_config.join(ELVISH_XDG_RC)],
        CompletionShell::Powershell => vec![xdg_config.join(POWERSHELL_RC)],
    }
}

/// The report over all probes: one labeled row each, closed by a summary
/// counting outcomes (shown in the interactive layout only), and failed when
/// a probe failed.
pub fn doctor_report(probes: &[Probe]) -> Report {
    let rows = probes.iter().map(probe_row).collect();
    Report::new(DOCTOR_TITLE, rows)
        .with_tally()
        .with_failure(any_failed(probes))
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

    /// A project whose config path is a dangling symlink is flagged as missing.
    #[test]
    fn registry_probe_flags_dangling_symlink_project() {
        use std::os::unix::fs::symlink;

        let dir =
            std::env::temp_dir().join(format!("muster-probe-dangling-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("muster.yml");
        symlink(dir.join("nonexistent.yml"), &config_path).unwrap();

        let registry = RecordingRegistry {
            projects: vec![project("dangling", config_path.to_str().unwrap())],
            ..RecordingRegistry::default()
        };
        let probe = registry_probe(&registry);

        assert_eq!(probe.outcome(), ProbeOutcome::Fail);
        assert!(
            probe.detail().contains("dangling"),
            "dangling project named in detail"
        );

        std::fs::remove_dir_all(dir).unwrap();
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

    /// A stale install, as one an older muster wrote, fails the probe and
    /// names the fix.
    #[test]
    fn hooks_probe_asks_for_setup_on_a_stale_install() {
        let statuses = vec![hook_status(AgentTool::Amp, HookState::Stale)];
        let probe = hooks_probe(&statuses);
        assert_eq!(probe.outcome(), ProbeOutcome::Fail);
        assert!(probe.detail().contains("stale"), "{}", probe.detail());
        assert!(
            probe.detail().contains("muster hooks setup"),
            "{}",
            probe.detail()
        );
    }

    /// All-installed hooks pass.
    #[test]
    fn hooks_probe_passes_when_everything_is_installed() {
        let statuses = vec![hook_status(AgentTool::Claude, HookState::Installed)];
        assert_eq!(hooks_probe(&statuses).outcome(), ProbeOutcome::Ok);
    }

    /// The completions probe warns with the exact line when unregistered.
    #[test]
    fn completions_probe_warns_with_the_hook_line() {
        let dir = std::env::temp_dir().join(format!("muster-doc-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".zshrc"), "# nothing here\n").unwrap();
        let probe = completions_probe(Some("/bin/zsh"), &dir, &dir.join(".config"));
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
        let probe = completions_probe(Some("/bin/zsh"), &dir, &dir.join(".config"));
        assert_eq!(probe.outcome(), ProbeOutcome::Warn);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// When the hook line is in the second fish location (completions file),
    /// the probe reports Ok and names that file.
    #[test]
    fn completions_probe_detects_fish_completions_file() {
        let dir = std::env::temp_dir().join(format!("muster-doc-fish-{}", uuid::Uuid::new_v4()));
        let completions_path = dir.join(".config").join(FISH_COMPLETIONS);
        std::fs::create_dir_all(completions_path.parent().unwrap()).unwrap();
        // Write only to the completions drop-in, not config.fish.
        std::fs::write(&completions_path, registration_line(CompletionShell::Fish)).unwrap();
        let probe = completions_probe(Some("/usr/bin/fish"), &dir, &dir.join(".config"));
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
        let rc_path = dir.join(".config").join(POWERSHELL_RC);
        std::fs::create_dir_all(rc_path.parent().unwrap()).unwrap();
        std::fs::write(&rc_path, registration_line(CompletionShell::Powershell)).unwrap();

        let probe = completions_probe(Some("/usr/bin/pwsh"), &dir, &dir.join(".config"));

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

    /// A commented-out registration line does not count as registered.
    #[test]
    fn completions_probe_ignores_commented_registrations() {
        let dir = std::env::temp_dir().join(format!("muster-doc-comment-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(".zshrc"),
            format!("# {}\n", registration_line(CompletionShell::Zsh)),
        )
        .unwrap();

        let probe = completions_probe(Some("/bin/zsh"), &dir, &dir.join(".config"));

        assert_eq!(probe.outcome(), ProbeOutcome::Warn);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A `$SHELL` path maps to a completion shell only when muster knows it.
    #[test]
    fn shell_from_path_names_known_shells_only() {
        assert_eq!(shell_from_path("/bin/zsh"), Some(CompletionShell::Zsh));
        assert_eq!(shell_from_path("/bin/tcsh"), None);
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
