use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use clap::ValueEnum;
use strum::IntoEnumIterator;

use super::{
    completions::{CompletionShell, registration_line},
    doctor_check::DoctorCheck,
    error::CliError,
    output::COLUMN_GAP,
    probe::Probe,
    probe_outcome::ProbeOutcome,
    report::Report,
    row::Row,
    row_kind::RowKind,
    shell_init::rc_line,
};
use crate::{
    adapter::{
        client::{ClientError, ProjectTarget, SocketDirCheck},
        sanitize::printable,
        shell::HookShell,
    },
    domain::{
        config::{ConfigError, ProcessSpec, Stack},
        port::{LocalState, ProjectStore, StackStore},
        value::Pid,
    },
};

/// Most of doctor's ancestors searched for the hooked shell: past any chain
/// of wrappers it runs under.
pub const HOOK_SEARCH_DEPTH: usize = 16;

/// Title of the doctor report.
const DOCTOR_TITLE: &str = "muster doctor";
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
/// What to do about projects whose stored rows do not read.
const UNREADABLE_HINT: &str = "`muster projects rm <folder> --yes` forgets them";
/// What to do about legacy files.
const LEGACY_HINT: &str = "muster no longer reads them; delete them";
/// The socket probe when every directory the service is reached through is
/// private.
const SOCKET_OK: &str = "private to you";
/// Opens the socket probe's warning about a published socket it passes
/// over; why follows.
const PUBLISHED_PASSED_OVER: &str = "the socket a service published is passed over: ";
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

/// The shell-hook probe when the shell doctor runs in loads the hook.
const HOOK_ACTIVE: &str = "active in this shell";
/// Names of the programs that are shells: the nearest of doctor's ancestors
/// running one is the shell it runs in, and wrappers such as `env`,
/// `timeout` or `mise` between them are passed over.
const SHELL_NAMES: &[&str] = &[
    "bash", "zsh", "fish", "sh", "dash", "ash", "ksh", "ksh93", "mksh", "oksh", "pdksh", "yash",
    "posh", "csh", "tcsh", "nu", "elvish", "xonsh", "pwsh", "osh", "ysh", "rc", "es",
];
/// What a login shell's name starts with.
const LOGIN_SHELL_MARK: char = '-';
/// The shell-hook probe when it does not, before the line that loads it.
const HOOK_MISSING: &str = "not loaded in this shell; add to its rc file:";

/// Builds one probe.
fn probe(check: DoctorCheck, outcome: ProbeOutcome, detail: String) -> Probe {
    Probe::builder()
        .check(check)
        .outcome(outcome)
        .detail(detail)
        .build()
}

/// `error` and each source it does not already show, joined.
pub fn error_chain(error: &dyn Error) -> String {
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
        Ok(Ok(_)) => probe(
            DoctorCheck::Database,
            ProbeOutcome::Ok,
            DATABASE_OK.to_string(),
        ),
        Ok(Err(error)) => probe(
            DoctorCheck::Database,
            ProbeOutcome::Fail,
            error_chain(&error),
        ),
        Err(error) => probe(
            DoctorCheck::Database,
            ProbeOutcome::Fail,
            error_chain(error),
        ),
    }
}

/// How many projects are registered, failing on any whose stored row does
/// not read, naming its folder, and warning of any whose folder is gone.
pub fn projects_probe(projects: &dyn ProjectStore) -> Probe {
    match projects.projects().and_then(|readable| {
        projects
            .unreadable_projects()
            .map(|unreadable| (readable, unreadable))
    }) {
        Ok((_, unreadable)) if !unreadable.is_empty() => {
            let named: Vec<String> = unreadable
                .iter()
                .map(|project| {
                    format!(
                        "{} ({})",
                        printable(&project.root().to_string_lossy()),
                        project.reason()
                    )
                })
                .collect();
            probe(
                DoctorCheck::Projects,
                ProbeOutcome::Fail,
                format!(
                    "unreadable: {}{DETAIL_SEPARATOR}{UNREADABLE_HINT}",
                    named.join(LIST_SEPARATOR)
                ),
            )
        },
        Ok((projects, _)) => {
            let gone: Vec<String> = projects
                .iter()
                .filter(|project| !project.root().as_ref().is_dir())
                .map(|project| project.name().to_string())
                .collect();
            if gone.is_empty() {
                probe(
                    DoctorCheck::Projects,
                    ProbeOutcome::Ok,
                    format!("{} registered", projects.len()),
                )
            } else {
                probe(
                    DoctorCheck::Projects,
                    ProbeOutcome::Warn,
                    format!(
                        "folder gone for: {}{DETAIL_SEPARATOR}{GONE_HINT}",
                        gone.join(LIST_SEPARATOR)
                    ),
                )
            }
        },
        Err(error) => probe(
            DoctorCheck::Projects,
            ProbeOutcome::Fail,
            error_chain(&error),
        ),
    }
}

/// Whether every registered project's stored processes read and may start,
/// naming each project whose rows do not and why, and each process never
/// started for a stored problem, with the commands that fix it.
pub fn processes_probe(projects: &dyn ProjectStore, stacks: &dyn StackStore) -> Probe {
    let projects = match projects.projects() {
        Ok(projects) => projects,
        Err(error) => {
            return probe(
                DoctorCheck::Processes,
                ProbeOutcome::Fail,
                error_chain(&error),
            );
        },
    };
    let mut invalid = Vec::new();
    let mut never_started = Vec::new();
    for project in &projects {
        match stacks.stack(project.root()) {
            Ok(stack) => never_started.extend(
                stack
                    .iter()
                    .flat_map(Stack::processes)
                    .filter_map(ProcessSpec::stored_problem)
                    .map(|problem| format!("in {}, {problem}", project.name())),
            ),
            Err(error) => invalid.push(format!("{} ({})", project.name(), error_chain(&error))),
        }
    }
    let mut problems = Vec::new();
    if !invalid.is_empty() {
        problems.push(format!(
            "invalid in: {}{DETAIL_SEPARATOR}{INVALID_HINT}",
            invalid.join(LIST_SEPARATOR)
        ));
    }
    problems.extend(never_started);
    if problems.is_empty() {
        probe(
            DoctorCheck::Processes,
            ProbeOutcome::Ok,
            PROCESSES_OK.to_string(),
        )
    } else {
        probe(
            DoctorCheck::Processes,
            ProbeOutcome::Fail,
            printable(&problems.join(DETAIL_SEPARATOR)),
        )
    }
}

/// Which project the current folder belongs to, as every command finds it.
pub fn project_probe(target: &Result<ProjectTarget, CliError>) -> Probe {
    match target {
        Ok(ProjectTarget::Registered(project)) => probe(
            DoctorCheck::ThisFolder,
            ProbeOutcome::Ok,
            format!(
                "{}{COLUMN_GAP}{}",
                project.name(),
                project.root().as_ref().display()
            ),
        ),
        Ok(ProjectTarget::Unregistered(_)) => probe(
            DoctorCheck::ThisFolder,
            ProbeOutcome::Ok,
            NO_PROJECT_HERE.to_string(),
        ),
        Err(error) => probe(
            DoctorCheck::ThisFolder,
            ProbeOutcome::Fail,
            error_chain(error),
        ),
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
        return probe(
            DoctorCheck::OldFiles,
            ProbeOutcome::Ok,
            NO_LEGACY_FILES.to_string(),
        );
    }
    let listed: Vec<String> = left.iter().map(|path| path.display().to_string()).collect();
    probe(
        DoctorCheck::OldFiles,
        ProbeOutcome::Warn,
        format!(
            "{}{DETAIL_SEPARATOR}{LEGACY_HINT}",
            listed.join(LIST_SEPARATOR)
        ),
    )
}

/// Whether the directories this user's service is reached through are
/// private: this client's own fails, since no request goes there, and a
/// published one that is not private warns, since it is passed over.
pub fn socket_probe(check: &Result<SocketDirCheck, ClientError>) -> Probe {
    let check = match check {
        Ok(check) => check,
        Err(error) => {
            return probe(DoctorCheck::Socket, ProbeOutcome::Fail, error_chain(error));
        },
    };
    if let Some(own) = check.own() {
        return probe(
            DoctorCheck::Socket,
            ProbeOutcome::Fail,
            printable(&own.to_string()),
        );
    }
    match check.published() {
        Some(published) => probe(
            DoctorCheck::Socket,
            ProbeOutcome::Warn,
            printable(&format!("{PUBLISHED_PASSED_OVER}{published}")),
        ),
        None => probe(DoctorCheck::Socket, ProbeOutcome::Ok, SOCKET_OK.to_string()),
    }
}

/// Best-effort check that the shell's rc file registers completions; warns
/// with the exact line to add when it does not.
pub fn completions_probe(shell_path: Option<&str>, home: &Path, xdg_config: &Path) -> Probe {
    let Some(shell) = shell_path.and_then(shell_from_path) else {
        return probe(
            DoctorCheck::Completions,
            ProbeOutcome::Warn,
            "unknown shell; see `muster completions --help`".to_string(),
        );
    };
    let matched = rc_paths(shell, home, xdg_config).into_iter().find(|path| {
        fs::read_to_string(path).is_ok_and(|content| registers_completions(&content, shell))
    });
    if let Some(matched) = matched {
        probe(
            DoctorCheck::Completions,
            ProbeOutcome::Ok,
            format!("registered in {}", matched.display()),
        )
    } else {
        probe(
            DoctorCheck::Completions,
            ProbeOutcome::Warn,
            format!("not registered; add: {}", registration_line(shell)),
        )
    }
}

/// Whether the shell the hook exported as `exported` is the nearest shell
/// among doctor's `ancestors`, each a pid and its process name, so doctor
/// runs in it even under `env`, `timeout` or `mise exec`, while a shell
/// started inside it without the hook, which inherits the exported pid, is
/// not taken for it. When it is not, warns with the line that loads the
/// hook for the shell `shell_path` names, or for every shell when it names
/// none the hook supports.
pub fn shell_hook_probe(
    exported: Option<&str>,
    ancestors: &[(Pid, String)],
    shell_path: Option<&str>,
) -> Probe {
    let nearest_shell = ancestors
        .iter()
        .find(|(_, name)| is_shell(name))
        .map(|(pid, _)| pid.into_inner());
    let hooked = exported
        .and_then(|pid| pid.trim().parse::<u32>().ok())
        .is_some_and(|pid| nearest_shell == Some(pid));
    if hooked {
        return probe(
            DoctorCheck::ShellHook,
            ProbeOutcome::Ok,
            HOOK_ACTIVE.to_string(),
        );
    }
    let lines: Vec<String> = match shell_path.and_then(hook_shell_from_path) {
        Some(shell) => vec![rc_line(shell)],
        None => HookShell::iter().map(rc_line).collect(),
    };
    probe(
        DoctorCheck::ShellHook,
        ProbeOutcome::Warn,
        format!("{HOOK_MISSING} {}", lines.join(DETAIL_SEPARATOR)),
    )
}

/// Whether a process named `name` runs a shell, a login shell's leading
/// mark aside.
fn is_shell(name: &str) -> bool {
    SHELL_NAMES.contains(&name.trim_start_matches(LOGIN_SHELL_MARK))
}

/// The hooked shell a `$SHELL` path names, if the hook supports it.
fn hook_shell_from_path(shell_path: &str) -> Option<HookShell> {
    Path::new(shell_path).file_name()?.to_str()?.parse().ok()
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
    Row::labeled(kind, probe.check().label(), probe.detail().clone())
}

/// Whether any probe failed (warnings do not fail the run).
pub fn any_failed(probes: &[Probe]) -> bool {
    probes
        .iter()
        .any(|probe| probe.outcome() == ProbeOutcome::Fail)
}

#[cfg(test)]
mod tests {
    use std::io;

    use tempfile::TempDir;

    use super::*;
    use crate::{
        adapter::{
            cli::{doctor_view::DoctorView, embedding::Embedding, wrapped::Wrapped},
            client::fixed_projects::FixedProjects,
        },
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
        let probe = projects_probe(&FixedProjects::new(vec![
            project("here", &here),
            project("gone", &here.join("gone")),
        ]));

        assert_eq!(probe.outcome(), ProbeOutcome::Warn);
        assert!(probe.detail().contains("gone"), "{}", probe.detail());
        let fine = projects_probe(&FixedProjects::new(vec![project("here", &here)]));
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
        assert_eq!(
            error_chain(&Embedding::new(Wrapped)),
            "could not open: locked"
        );
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

    /// The ancestors `raw` names, each a pid and its process name.
    fn pids(raw: &[(u32, &str)]) -> Vec<(Pid, String)> {
        raw.iter()
            .map(|(pid, name)| (Pid::try_new(*pid).unwrap(), (*name).to_string()))
            .collect()
    }

    /// The hook is found only when the shell that exported its pid is the
    /// nearest shell among doctor's ancestors, its parent or past wrappers
    /// such as `env`, `timeout` or `mise exec`; otherwise the probe warns
    /// with the line for `$SHELL`, or every shell's line when `$SHELL` is
    /// none the hook supports.
    #[test]
    fn doctor_finds_the_hook_only_when_the_nearest_shell_is_the_hooked_one() {
        for ancestors in [
            &[(4242, "zsh"), (300, "systemd")][..],
            &[(4300, "env"), (4242, "bash"), (300, "systemd")],
            &[(4301, "timeout"), (4300, "mise"), (4242, "-zsh")],
        ] {
            let active = shell_hook_probe(Some("4242"), &pids(ancestors), Some("/bin/zsh"));
            assert_eq!(active.outcome(), ProbeOutcome::Ok, "{ancestors:?}");
            assert_eq!(active.detail(), HOOK_ACTIVE);
            assert_eq!(active.check(), DoctorCheck::ShellHook);
        }

        for (exported, ancestors) in [
            (Some("4242"), &[(4243, "zsh"), (300, "systemd")][..]),
            (None, &[(4242, "zsh")]),
            (Some("4242"), &[]),
            (Some("4242"), &[(4242, "env")]),
            (Some("not a pid"), &[(4242, "zsh")]),
        ] {
            let probe = shell_hook_probe(exported, &pids(ancestors), Some("/usr/bin/zsh"));
            assert_eq!(
                probe.outcome(),
                ProbeOutcome::Warn,
                "{exported:?} {ancestors:?}"
            );
            assert_eq!(
                probe.detail(),
                &format!("{HOOK_MISSING} {}", rc_line(HookShell::Zsh))
            );
        }

        let fish = shell_hook_probe(None, &pids(&[(2, "fish")]), Some("/opt/homebrew/bin/fish"));
        assert!(
            fish.detail().ends_with("muster shell-init fish | source"),
            "{}",
            fish.detail()
        );

        for shell_path in [Some("/bin/tcsh"), None] {
            let unknown = shell_hook_probe(None, &pids(&[(4242, "tcsh")]), shell_path);
            assert_eq!(unknown.outcome(), ProbeOutcome::Warn);
            for shell in HookShell::iter() {
                assert!(
                    unknown.detail().contains(&rc_line(shell)),
                    "{}",
                    unknown.detail()
                );
            }
        }
    }

    /// CR-3: a shell started inside the hooked one without the hook, as
    /// `bash --norc` is, inherits the exported pid but is the nearest
    /// shell, so doctor says the hook is not loaded there, under a wrapper
    /// or not.
    #[test]
    fn an_unhooked_shell_inside_a_hooked_one_is_not_hooked() {
        for ancestors in [
            &[(5000, "bash"), (4242, "bash"), (300, "systemd")][..],
            &[(5001, "env"), (5000, "bash"), (4242, "zsh")],
            &[(5000, "sh"), (4999, "timeout"), (4242, "bash")],
        ] {
            let probe = shell_hook_probe(Some("4242"), &pids(ancestors), Some("/bin/bash"));
            assert_eq!(probe.outcome(), ProbeOutcome::Warn, "{ancestors:?}");
        }
    }

    /// A process is a shell by its name, a login shell's leading mark
    /// aside; wrappers and other programs are not.
    #[test]
    fn shells_are_told_by_name() {
        for shell in ["bash", "zsh", "fish", "sh", "dash", "ksh", "-bash", "-zsh"] {
            assert!(is_shell(shell), "{shell}");
        }
        for other in ["env", "timeout", "mise", "muster", "sudo", ""] {
            assert!(!is_shell(other), "{other}");
        }
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

    /// The socket probe fails on a squatted own socket directory, warns
    /// naming a published one it passes over, and is fine otherwise.
    #[test]
    fn the_socket_probe_names_a_squatted_directory() {
        let squat = |dir: &str| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("the socket directory {dir} is not a directory owned by you"),
            )
        };
        let checked = |published, own| {
            Ok(SocketDirCheck::builder()
                .published(published)
                .own(own)
                .build())
        };

        let own = socket_probe(&checked(None, Some(squat("/tmp/muster-1000"))));
        let published = socket_probe(&checked(Some(squat("/old/run/muster")), None));
        let fine = socket_probe(&checked(None, None));

        assert_eq!(own.outcome(), ProbeOutcome::Fail);
        assert!(
            own.detail().contains("/tmp/muster-1000"),
            "{}",
            own.detail()
        );
        assert_eq!(published.outcome(), ProbeOutcome::Warn);
        assert!(
            published.detail().contains("passed over")
                && published.detail().contains("/old/run/muster"),
            "{}",
            published.detail()
        );
        assert_eq!(fine.outcome(), ProbeOutcome::Ok);
    }

    /// The doctor report maps probes to labeled rows and counts the summary.
    #[test]
    fn doctor_report_maps_probes_and_summarizes() {
        let probes = vec![
            probe(DoctorCheck::Database, ProbeOutcome::Ok, "fine".to_string()),
            probe(
                DoctorCheck::Completions,
                ProbeOutcome::Warn,
                "not registered".to_string(),
            ),
        ];

        let report = doctor_report(&probes);

        assert_eq!(report.rows().len(), 2);
        assert_eq!(report.rows()[0].kind(), RowKind::Ok);
        assert_eq!(report.rows()[0].label().as_deref(), Some("database"));
        assert_eq!(report.rows()[1].kind(), RowKind::Hint);
        assert_eq!(report.summary().as_deref(), Some("1 ok · 1 hint"));
    }

    /// `--json` names each check and outcome in kebab case and says whether
    /// one failed.
    #[test]
    fn the_json_names_checks_and_outcomes() {
        let probes = vec![
            probe(
                DoctorCheck::ThisFolder,
                ProbeOutcome::Ok,
                "here".to_string(),
            ),
            probe(DoctorCheck::OldFiles, ProbeOutcome::Fail, "bad".to_string()),
        ];
        let view = DoctorView::builder()
            .checks(&probes)
            .failed(any_failed(&probes))
            .build();

        assert_eq!(
            serde_json::to_value(&view).unwrap(),
            serde_json::json!({
                "checks": [
                    {"check": "this-folder", "outcome": "ok", "detail": "here"},
                    {"check": "old-files", "outcome": "fail", "detail": "bad"},
                ],
                "failed": true,
            })
        );
    }
}
