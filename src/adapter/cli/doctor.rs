use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use clap::ValueEnum;
use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::{
    completions::{CompletionShell, registration_line},
    error::CliError,
    output::COLUMN_GAP,
    probe_outcome::ProbeOutcome,
    report::Report,
    row::Row,
    row_kind::RowKind,
};
use crate::{
    adapter::client::ProjectTarget,
    domain::{
        config::ConfigError,
        port::{LocalState, ProjectStore, StackStore},
    },
};

/// Title of the doctor report.
const DOCTOR_TITLE: &str = "muster doctor";
/// Probe labels.
const DATABASE_LABEL: &str = "database";
const PROJECTS_LABEL: &str = "projects";
const PROCESSES_LABEL: &str = "processes";
const PROJECT_LABEL: &str = "this folder";
const LEGACY_LABEL: &str = "old files";
const COMPLETIONS_LABEL: &str = "completions";
/// The database probe when the database opens and its settings read.
const DATABASE_OK: &str = "opens at the current schema, settings valid";
/// The current-folder probe outside every project.
const NO_PROJECT_HERE: &str = "not inside a project; `muster add` starts one";
/// The legacy probe with nothing left over.
const NO_LEGACY_FILES: &str = "none left over";
/// The processes probe when every stored process reads.
const PROCESSES_OK: &str = "every stored process reads";
/// What to do about projects whose stored processes do not read.
const INVALID_HINT: &str = "`muster projects rm <name> --yes` forgets them";
/// What to do about registered folders that are gone.
const GONE_HINT: &str = "`muster projects rm <name>` forgets them";
/// What to do about legacy files.
const LEGACY_HINT: &str = "muster no longer reads them; delete them";
/// Between the parts of a probe's detail.
const DETAIL_SEPARATOR: &str = "; ";
/// Between the items a probe lists.
const LIST_SEPARATOR: &str = ", ";
/// Between the messages of an error and its sources.
const CHAIN_SEPARATOR: &str = ": ";
/// Files an older muster kept in the state directory.
const LEGACY_STATE_FILES: &[&str] = &["coordination.db", "agent-sessions.yml"];
/// Files an older muster kept in the config directory.
const LEGACY_CONFIG_FILES: &[&str] = &["settings.yml", "projects.yml"];
/// Prefix of a commented-out rc line, ignored by every probed shell.
const COMMENT_PREFIX: &str = "#";
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

/// `error` and each source it does not already show, joined.
fn error_chain(error: &dyn Error) -> String {
    let mut report = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        if !report.contains(&text) {
            report.push_str(CHAIN_SEPARATOR);
            report.push_str(&text);
        }
        source = cause.source();
    }
    report
}

/// Whether the database `opened` and its settings row reads.
pub fn database_probe(opened: &Result<Arc<dyn LocalState>, ConfigError>) -> Probe {
    match opened.as_ref().map(|state| state.load()) {
        Ok(Ok(_)) => probe(DATABASE_LABEL, ProbeOutcome::Ok, DATABASE_OK.to_string()),
        Ok(Err(error)) => probe(DATABASE_LABEL, ProbeOutcome::Fail, error_chain(&error)),
        Err(error) => probe(DATABASE_LABEL, ProbeOutcome::Fail, error_chain(error)),
    }
}

/// How many projects are registered, warning of any whose folder is gone.
pub fn projects_probe(projects: &dyn ProjectStore) -> Probe {
    match projects.projects() {
        Ok(projects) => {
            let gone: Vec<String> = projects
                .iter()
                .filter(|project| !project.root().as_ref().is_dir())
                .map(|project| project.name().to_string())
                .collect();
            if gone.is_empty() {
                probe(
                    PROJECTS_LABEL,
                    ProbeOutcome::Ok,
                    format!("{} registered", projects.len()),
                )
            } else {
                probe(
                    PROJECTS_LABEL,
                    ProbeOutcome::Warn,
                    format!(
                        "folder gone for: {}{DETAIL_SEPARATOR}{GONE_HINT}",
                        gone.join(LIST_SEPARATOR)
                    ),
                )
            }
        },
        Err(error) => probe(PROJECTS_LABEL, ProbeOutcome::Fail, error_chain(&error)),
    }
}

/// Whether every registered project's stored processes read, naming each
/// project whose rows do not and why.
pub fn processes_probe(projects: &dyn ProjectStore, stacks: &dyn StackStore) -> Probe {
    let projects = match projects.projects() {
        Ok(projects) => projects,
        Err(error) => return probe(PROCESSES_LABEL, ProbeOutcome::Fail, error_chain(&error)),
    };
    let invalid: Vec<String> = projects
        .iter()
        .filter_map(|project| {
            stacks
                .stack(project.root())
                .err()
                .map(|error| format!("{} ({})", project.name(), error_chain(&error)))
        })
        .collect();
    if invalid.is_empty() {
        probe(PROCESSES_LABEL, ProbeOutcome::Ok, PROCESSES_OK.to_string())
    } else {
        probe(
            PROCESSES_LABEL,
            ProbeOutcome::Fail,
            format!(
                "invalid in: {}{DETAIL_SEPARATOR}{INVALID_HINT}",
                invalid.join(LIST_SEPARATOR)
            ),
        )
    }
}

/// Which project the current folder belongs to, as every command finds it.
pub fn project_probe(target: &Result<ProjectTarget, CliError>) -> Probe {
    match target {
        Ok(ProjectTarget::Registered(project)) => probe(
            PROJECT_LABEL,
            ProbeOutcome::Ok,
            format!(
                "{}{COLUMN_GAP}{}",
                project.name(),
                project.root().as_ref().display()
            ),
        ),
        Ok(ProjectTarget::Unregistered(_)) => {
            probe(PROJECT_LABEL, ProbeOutcome::Ok, NO_PROJECT_HERE.to_string())
        },
        Err(error) => probe(PROJECT_LABEL, ProbeOutcome::Fail, error_chain(error)),
    }
}

/// Files an older muster left in `state_dir` and `config_dir`, which this
/// one never reads.
pub fn legacy_probe(state_dir: Option<&Path>, config_dir: Option<&Path>) -> Probe {
    let left: Vec<PathBuf> = [
        (state_dir, LEGACY_STATE_FILES),
        (config_dir, LEGACY_CONFIG_FILES),
    ]
    .into_iter()
    .filter_map(|(dir, files)| dir.map(|dir| (dir, files)))
    .flat_map(|(dir, files)| files.iter().map(move |file| dir.join(file)))
    .filter(|path| fs::symlink_metadata(path).is_ok())
    .collect();
    if left.is_empty() {
        return probe(LEGACY_LABEL, ProbeOutcome::Ok, NO_LEGACY_FILES.to_string());
    }
    let listed: Vec<String> = left.iter().map(|path| path.display().to_string()).collect();
    probe(
        LEGACY_LABEL,
        ProbeOutcome::Warn,
        format!(
            "{}{DETAIL_SEPARATOR}{LEGACY_HINT}",
            listed.join(LIST_SEPARATOR)
        ),
    )
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
    CompletionShell::from_str(name, false).ok()
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
    use tempfile::TempDir;

    use super::*;
    use crate::{
        adapter::cli::{embedding::Embedding, fixed_projects::Projects, wrapped::Wrapped},
        domain::{
            project::Project,
            value::{ProjectName, ProjectRoot},
        },
    };

    /// A project called `name` at `root`.
    fn project(name: &str, root: &Path) -> Project {
        Project::builder()
            .root(ProjectRoot::try_new(root.to_path_buf()).unwrap())
            .name(ProjectName::try_new(name).unwrap())
            .build()
    }

    /// A registered project whose folder was deleted is named as a warning,
    /// never a failure.
    #[test]
    fn a_registered_folder_that_is_gone_is_a_warning() {
        let scratch = TempDir::new().unwrap();
        let here = scratch.path().canonicalize().unwrap();
        let probe = projects_probe(&Projects(vec![
            project("here", &here),
            project("gone", &here.join("gone")),
        ]));

        assert_eq!(probe.outcome(), ProbeOutcome::Warn);
        assert!(probe.detail().contains("gone"), "{}", probe.detail());
        let fine = projects_probe(&Projects(vec![project("here", &here)]));
        assert_eq!(fine.outcome(), ProbeOutcome::Ok);
        assert_eq!(fine.detail(), "1 registered");
    }

    /// A database that does not open fails the doctor, naming why.
    #[test]
    fn a_database_that_does_not_open_fails() {
        let probe = database_probe(&Err(ConfigError::UnsupportedSchema {
            found: 9,
            supported: 1,
        }));

        assert_eq!(probe.outcome(), ProbeOutcome::Fail);
        assert!(
            probe.detail().contains("upgrade muster"),
            "{}",
            probe.detail()
        );
    }

    /// Files an older muster left are named for deletion.
    #[test]
    fn leftover_files_are_named() {
        let state = TempDir::new().unwrap();
        let config = TempDir::new().unwrap();
        assert_eq!(
            legacy_probe(Some(state.path()), Some(config.path())).outcome(),
            ProbeOutcome::Ok
        );
        fs::write(state.path().join("coordination.db"), "").unwrap();
        fs::write(config.path().join("settings.yml"), "").unwrap();

        let probe = legacy_probe(Some(state.path()), Some(config.path()));
        assert_eq!(probe.outcome(), ProbeOutcome::Warn);
        assert!(probe.detail().contains("coordination.db"));
        assert!(probe.detail().contains("settings.yml"));
    }

    /// The current folder's project is named; outside one it is not a
    /// failure.
    #[test]
    fn the_current_project_is_named() {
        let root = ProjectRoot::try_new(PathBuf::from("/home/me/shop")).unwrap();
        let shop = project("shop", root.as_ref());

        assert_eq!(
            project_probe(&Ok(ProjectTarget::Registered(shop))).detail(),
            "shop  /home/me/shop"
        );
        assert_eq!(
            project_probe(&Ok(ProjectTarget::Unregistered(root))).outcome(),
            ProbeOutcome::Ok
        );
    }

    /// A source the message already shows is not repeated.
    #[test]
    fn the_error_chain_skips_sources_already_shown() {
        assert_eq!(error_chain(&Embedding(Wrapped)), "could not open: locked");
    }

    /// The completions probe warns with the exact line when unregistered.
    #[test]
    fn completions_probe_warns_with_the_hook_line() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(".zshrc"), "# nothing here\n").unwrap();
        let probe = completions_probe(Some("/bin/zsh"), dir.path(), &dir.path().join(".config"));
        assert_eq!(probe.outcome(), ProbeOutcome::Warn);
        assert!(probe.detail().contains("COMPLETE=zsh"));
    }

    /// An rc file that mentions COMPLETE and muster in unrelated contexts must
    /// NOT be reported as registered.
    #[test]
    fn completions_probe_ignores_unrelated_complete_mention() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join(".zshrc"),
            "# COMPLETE list for muster tasks\n",
        )
        .unwrap();
        let probe = completions_probe(Some("/bin/zsh"), dir.path(), &dir.path().join(".config"));
        assert_eq!(probe.outcome(), ProbeOutcome::Warn);
    }

    /// When the hook line is in the second fish location (completions file),
    /// the probe reports Ok and names that file.
    #[test]
    fn completions_probe_detects_fish_completions_file() {
        let dir = TempDir::new().unwrap();
        let completions_path = dir.path().join(".config").join(FISH_COMPLETIONS);
        fs::create_dir_all(completions_path.parent().unwrap()).unwrap();
        fs::write(&completions_path, registration_line(CompletionShell::Fish)).unwrap();
        let probe = completions_probe(
            Some("/usr/bin/fish"),
            dir.path(),
            &dir.path().join(".config"),
        );
        assert_eq!(probe.outcome(), ProbeOutcome::Ok);
        assert!(probe.detail().contains("completions/muster.fish"));
    }

    /// The completions probe detects a PowerShell profile containing the exact
    /// registration line, including the spaces around `=` that would foil a
    /// `COMPLETE=` needle.
    #[test]
    fn completions_probe_detects_powershell_registration() {
        let dir = TempDir::new().unwrap();
        let rc_path = dir.path().join(".config").join(POWERSHELL_RC);
        fs::create_dir_all(rc_path.parent().unwrap()).unwrap();
        fs::write(&rc_path, registration_line(CompletionShell::Powershell)).unwrap();

        let probe = completions_probe(
            Some("/usr/bin/pwsh"),
            dir.path(),
            &dir.path().join(".config"),
        );

        assert_eq!(probe.outcome(), ProbeOutcome::Ok);
    }

    /// A commented-out registration line does not count as registered.
    #[test]
    fn completions_probe_ignores_commented_registrations() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join(".zshrc"),
            format!("# {}\n", registration_line(CompletionShell::Zsh)),
        )
        .unwrap();

        let probe = completions_probe(Some("/bin/zsh"), dir.path(), &dir.path().join(".config"));

        assert_eq!(probe.outcome(), ProbeOutcome::Warn);
    }

    /// A `$SHELL` path maps to a completion shell only when muster knows it.
    #[test]
    fn shell_from_path_names_known_shells_only() {
        assert_eq!(shell_from_path("/bin/zsh"), Some(CompletionShell::Zsh));
        assert_eq!(
            shell_from_path("/usr/bin/pwsh"),
            Some(CompletionShell::Powershell)
        );
        assert_eq!(shell_from_path("/bin/tcsh"), None);
    }

    /// The doctor report maps probes to labeled rows and counts the summary.
    #[test]
    fn doctor_report_maps_probes_and_summarizes() {
        let probes = vec![
            probe(DATABASE_LABEL, ProbeOutcome::Ok, "fine".to_string()),
            probe(
                COMPLETIONS_LABEL,
                ProbeOutcome::Warn,
                "not registered".to_string(),
            ),
        ];

        let report = doctor_report(&probes);

        assert_eq!(report.rows().len(), 2);
        assert_eq!(report.rows()[0].kind(), RowKind::Ok);
        assert_eq!(report.rows()[0].label().as_deref(), Some(DATABASE_LABEL));
        assert_eq!(report.rows()[1].kind(), RowKind::Hint);
        assert_eq!(report.summary().as_deref(), Some("1 ok · 1 hint"));
    }
}
