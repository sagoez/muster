use std::{error::Error, fmt::Display, str::FromStr, sync::LazyLock};

use clap::{
    Command as ClapCommand, CommandFactory, Parser,
    builder::{
        PossibleValuesParser, TypedValueParser,
        styling::{AnsiColor, Styles},
    },
};
use clap_complete::{ArgValueCandidates, CompletionCandidate};
use getset::Getters;
use strum::IntoEnumIterator;

use super::{cli_ports::CliPorts, command::Command};
use crate::{
    adapter::{
        holder::{LineLimit, MAX_LOG_LINES},
        sanitize::printable,
        service::StateGoal,
        shell::HookShell,
    },
    constants::APP_NAME,
    domain::{
        config::SettingKey,
        process::{RestartPolicy, StopSignal},
        value::{Pid, PositiveDuration, StopGrace},
    },
};

/// The id of the global `--project` argument.
const PROJECT_ARG: &str = "project";
/// Help styling matching the CLI's report palette: cyan structure, green
/// literals, yellow placeholders.
const HELP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Cyan.on_default().bold())
    .usage(AnsiColor::Cyan.on_default().bold())
    .literal(AnsiColor::Green.on_default())
    .placeholder(AnsiColor::Yellow.on_default());

/// What the main help says after the commands: the exit codes and the JSON
/// conventions every command keeps. Tests pin the codes to
/// [`CliExit`](super::CliExit) and to the README's table.
const AFTER_HELP: &str = "\
Exit codes:
    0  success
    1  a process did not end up as asked, the action failed, or the current state refused it (a taken name, an import conflict, a folder holding a project, a todo another agent holds)
    2  the command line could not be parsed, set named nothing to change, a confirmation is missing, or a question was cancelled
    3  the muster service is unavailable or runs a newer version
    4  an invalid value: not a project, an unknown project or process, a refused argument, or an unusable file
  130  interrupted with Ctrl-C while it waited at a terminal

With --json, results and failures print as JSON on stdout. Object keys are
snake_case, enum values are kebab-case, durations are seconds (an idle_stop
that never comes is null), and times are RFC 3339 in UTC.";

/// What `muster logs --lines` takes, followed by its maximum.
const LINE_LIMIT_FORM: &str = "a whole number from 1 to ";
/// The `muster logs` flags that print the whole raw log or follow it, which
/// a filtered read conflicts with.
pub(super) const RAW_LOG_FLAGS: [&str; 2] = ["follow", "screen"];

/// The names `config` accepts for a setting.
pub(super) static SETTING_KEYS: LazyLock<Vec<String>> = LazyLock::new(variant_names::<SettingKey>);
/// The names `--restart` accepts.
static RESTART_POLICIES: LazyLock<Vec<String>> = LazyLock::new(variant_names::<RestartPolicy>);
/// The names `--stop-signal` accepts.
static STOP_SIGNALS: LazyLock<Vec<String>> = LazyLock::new(variant_names::<StopSignal>);
/// The names `muster wait --state` accepts.
static STATE_GOALS: LazyLock<Vec<String>> = LazyLock::new(variant_names::<StateGoal>);
/// The shells `muster shell-init` prints a hook for.
pub(super) static HOOK_SHELLS: LazyLock<Vec<String>> = LazyLock::new(variant_names::<HookShell>);
/// What a pid argument takes.
const PID_FORM: &str = "a process id above 1";

/// The kebab-case name of each variant of `T`.
fn variant_names<T: IntoEnumIterator + Display>() -> Vec<String> {
    T::iter().map(|variant| variant.to_string()).collect()
}

/// Parses one of `names` into `T`, listing them all in help and on a typo.
pub(super) fn one_of<T>(names: &'static [String]) -> impl TypedValueParser<Value = T>
where
    T: FromStr + Clone + Send + Sync + 'static,
    T::Err: Error + Send + Sync + 'static,
{
    PossibleValuesParser::new(names.iter().map(String::as_str)).try_map(|raw| raw.parse::<T>())
}

/// Parses a `--restart` policy.
pub(super) fn restart_parser() -> impl TypedValueParser<Value = RestartPolicy> {
    one_of(&RESTART_POLICIES)
}

/// Parses a `--stop-signal`.
pub(super) fn stop_signal_parser() -> impl TypedValueParser<Value = StopSignal> {
    one_of(&STOP_SIGNALS)
}

/// Parses a `muster wait --state`.
pub(super) fn state_goal_parser() -> impl TypedValueParser<Value = StateGoal> {
    one_of(&STATE_GOALS)
}

/// Parses a positive duration such as `5s` or `1m 30s`.
///
/// # Errors
/// Returns the parse failure of anything else.
pub(super) fn positive_duration(raw: &str) -> Result<PositiveDuration, String> {
    raw.parse::<PositiveDuration>()
        .map_err(|error| error.to_string())
}

/// Parses a stop grace such as `5s`, at most an hour.
///
/// # Errors
/// Returns the parse failure of anything else.
pub(super) fn stop_grace(raw: &str) -> Result<StopGrace, String> {
    raw.parse::<StopGrace>().map_err(|error| error.to_string())
}

/// Parses a `muster logs --lines`: a whole number from 1 to
/// [`MAX_LOG_LINES`].
///
/// # Errors
/// Returns the form expected for anything else.
pub(super) fn line_limit(raw: &str) -> Result<LineLimit, String> {
    raw.trim()
        .parse()
        .ok()
        .and_then(|lines| LineLimit::try_new(lines).ok())
        .ok_or_else(|| format!("{LINE_LIMIT_FORM}{MAX_LOG_LINES}"))
}

/// Parses a process id.
///
/// # Errors
/// Returns the form expected for anything but a whole number above 1.
pub(super) fn pid(raw: &str) -> Result<Pid, String> {
    raw.trim()
        .parse()
        .ok()
        .and_then(|pid| Pid::try_new(pid).ok())
        .ok_or_else(|| PID_FORM.to_string())
}

/// The command line's definition, with `--project` completing the projects
/// `ports` has registered.
pub(super) fn completing_command(ports: &'static dyn CliPorts) -> ClapCommand {
    Args::command().mut_arg(PROJECT_ARG, |arg| {
        arg.add(ArgValueCandidates::new(move || project_candidates(ports)))
    })
}

/// Shell-completion candidates for `--project`: the registered projects'
/// names, read fresh each time so a new project completes at once, each made
/// [`printable`] so the shell never echoes a terminal sequence. Reads an
/// existing database read-only and never creates one.
fn project_candidates(ports: &dyn CliPorts) -> Vec<CompletionCandidate> {
    ports
        .existing_state()
        .ok()
        .flatten()
        .and_then(|state| state.projects().ok())
        .unwrap_or_default()
        .iter()
        .map(|project| CompletionCandidate::new(printable(project.name().as_ref())))
        .collect()
}

/// Command-line arguments. With no subcommand, muster prints `ps` for the
/// current project, or every loaded project outside one.
#[derive(Parser, Getters)]
#[command(
    name = APP_NAME,
    bin_name = APP_NAME,
    version,
    about = "Run a project's dev processes in the background, and control them from any shell",
    after_help = AFTER_HELP,
    styles = HELP_STYLES
)]
pub struct Args {
    /// The project: a registered project's name, or a folder (a value with
    /// `/`, or starting with `.` or `~`). Defaults to $MUSTER_PROJECT, then
    /// the registered project containing the current folder.
    // Global, so it is recognized before or after a subcommand rather than
    // being swallowed by `run`'s command words.
    #[arg(short, long, global = true, value_name = "PROJECT")]
    #[getset(get = "pub")]
    project: Option<String>,
    /// The command to run; none prints `ps`.
    #[command(subcommand)]
    command: Option<Command>,
}

impl Args {
    /// The command to run, taken out of the arguments to be dispatched.
    pub fn into_command(self) -> Option<Command> {
        self.command
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        adapter::{
            holder::HOLDER_SUBCOMMAND, service::protocol::SERVICE_SUBCOMMAND,
            shell::SHELL_REPORT_SUBCOMMAND,
        },
        domain::occupancy::{LoadStart, ReportSeq},
    };

    /// Completions and usage must register the binary name, not the package
    /// name muster-workspace, or every emitted shell hook is inert.
    #[test]
    fn the_command_is_named_after_the_binary() {
        assert_eq!(Args::command().get_name(), APP_NAME);
        assert_eq!(Args::command().get_bin_name(), Some(APP_NAME));
        assert_eq!(
            Args::command().get_version(),
            Some(env!("CARGO_PKG_VERSION")),
            "muster --version must work on installed builds"
        );
    }

    /// Appends the help of `command`, invoked as `invocation`, and of each of
    /// its visible subcommands to `helps`.
    fn collect_help(command: &mut ClapCommand, invocation: &str, helps: &mut Vec<String>) {
        helps.push(format!("$ {invocation} --help\n{}", command.render_help()));
        for sub in command
            .get_subcommands_mut()
            .filter(|sub| !sub.is_hide_set() && sub.get_name() != "help")
        {
            let invocation = format!("{invocation} {}", sub.get_name());
            collect_help(sub, &invocation, helps);
        }
    }

    /// Every visible command's help, so a changed flag, name or description
    /// shows up in review.
    #[test]
    fn cli_args_snapshot() {
        let mut command = Args::command();
        command.build();
        let mut helps = Vec::new();
        collect_help(&mut command, APP_NAME, &mut helps);

        insta::assert_snapshot!(helps.join("\n\n"));
    }

    /// clap's own consistency checks pass for every command.
    #[test]
    fn the_command_line_is_consistent() {
        Args::command().debug_assert();
    }

    /// A name with spaces or an accent reaches a lifecycle command whole and
    /// in normal form C.
    #[test]
    fn names_with_spaces_and_unicode_parse_as_one_argument() {
        let args = Args::try_parse_from(["muster", "stop", "dev server", "--json"]).unwrap();
        let Some(Command::Stop(stop)) = args.command else {
            panic!("stop parses");
        };
        assert!(stop.json());
        assert_eq!(stop.name().as_ref().map(AsRef::as_ref), Some("dev server"));
        let args = Args::try_parse_from(["muster", "logs", "cafe\u{301}", "-f"]).unwrap();
        assert!(matches!(
            args.command,
            Some(Command::Logs { name: Some(name), follow: true, .. }) if name.as_ref() == "caf\u{e9}"
        ));
        assert!(Args::try_parse_from(["muster", "start", "  "]).is_err());
        assert!(Args::try_parse_from(["muster", "logs", "api", "-f", "--screen"]).is_err());
    }

    /// A lifecycle, log or attach command may leave its name out, to be
    /// chosen from a list; `down` takes no agent flags any more.
    #[test]
    fn a_name_may_be_left_to_choose() {
        for command in ["start", "stop", "restart", "logs", "attach"] {
            assert!(
                Args::try_parse_from(["muster", command]).is_ok(),
                "{command}"
            );
        }
        assert!(Args::try_parse_from(["muster", "down", "--agents"]).is_err());
        assert!(Args::try_parse_from(["muster", "rm"]).is_err());
    }

    /// `add` takes its command after `--` and the shared process flags;
    /// `set` takes the command as a flag or after `--`, never both.
    #[test]
    fn add_and_set_parse_their_definitions() {
        let args = Args::try_parse_from([
            "muster",
            "add",
            "web",
            "--env",
            "PORT=5173",
            "--restart",
            "on-failure",
            "--stop-grace",
            "10s",
            "--no-autostart",
            "--",
            "npm",
            "run",
            "dev",
        ])
        .unwrap();
        let Some(Command::Add(add)) = args.command else {
            panic!("add parses");
        };
        assert_eq!(add.name(), "web");
        assert_eq!(add.command(), &["npm", "run", "dev"]);
        assert_eq!(add.flags().restart(), Some(RestartPolicy::OnFailure));
        assert_eq!(add.flags().autostart(), Some(false));
        assert!(Args::try_parse_from(["muster", "add", "web"]).is_err());
        assert!(
            Args::try_parse_from(["muster", "add", "web", "--restart", "sometimes", "--", "x"])
                .is_err()
        );
        assert!(
            Args::try_parse_from(["muster", "set", "web", "--command", "a", "--", "b"]).is_err()
        );
        assert!(Args::try_parse_from(["muster", "set", "web", "--cwd", "x", "--no-cwd"]).is_err());
    }

    /// A misspelt setting lists every valid key; `config` alone lists them.
    #[test]
    fn config_keys_are_listed_on_a_typo() {
        let error = match Args::try_parse_from(["muster", "config", "get", "detach_key"]) {
            Err(error) => error.render().to_string(),
            Ok(_) => panic!("a typo parses"),
        };
        for key in SettingKey::iter() {
            assert!(error.contains(&key.to_string()), "{error}");
        }
        let args = Args::try_parse_from(["muster", "config", "--json"]).unwrap();
        assert!(matches!(
            args.command,
            Some(Command::Config {
                command: None,
                json: true
            })
        ));
    }

    /// `import` takes one of `--merge` and `--replace`, not both.
    #[test]
    fn import_modes_exclude_each_other() {
        assert!(Args::try_parse_from(["muster", "import", "--merge", "--replace"]).is_err());
        assert!(Args::try_parse_from(["muster", "import", "-", "--replace", "--dry-run"]).is_ok());
    }

    /// The exit codes the help documents are the ones commands exit with.
    #[test]
    fn the_documented_exit_codes_match() {
        use crate::adapter::cli::CliExit;

        for exit in CliExit::iter() {
            assert!(
                AFTER_HELP.contains(&format!("\n{:>5}  ", exit.code())),
                "{exit:?} is documented, aligned with the others"
            );
        }
        assert_eq!(
            AFTER_HELP.matches("\n  ").count(),
            CliExit::iter().count(),
            "no undocumented code is listed"
        );
    }

    /// The README's exit code table says what the help says, code for code.
    #[test]
    fn the_readme_documents_the_same_exit_codes() {
        let readme = include_str!("../../../README.md");

        for line in AFTER_HELP.lines().filter(|line| line.starts_with("  ")) {
            let (code, meaning) = line.trim_start().split_once("  ").unwrap();
            let mut letters = meaning.chars();
            let first = letters.next().unwrap().to_uppercase();
            let row = format!("| {code} | {first}{} |", letters.as_str());
            assert!(readme.contains(&row), "the README lacks {row}");
        }
    }

    /// The global project flag parses before and after a subcommand, and is
    /// not swallowed by `run`'s command words.
    #[test]
    fn the_project_flag_parses_anywhere() {
        let before = Args::try_parse_from(["muster", "-p", "shop", "run", "--", "ls"]).unwrap();
        assert_eq!(before.project.as_deref(), Some("shop"));
        let after = Args::try_parse_from(["muster", "ps", "--project", "./web"]).unwrap();
        assert_eq!(after.project.as_deref(), Some("./web"));
        let bare = Args::try_parse_from(["muster"]).unwrap();
        assert!(bare.command.is_none());
    }

    /// The hidden shell report takes the `--pid`, `--load`, `--load-pid`
    /// and `--seq` the hook passes, and refuses a pid no shell could have or
    /// a load that is no number.
    #[test]
    fn the_shell_report_reads_the_hook_s_pid_load_and_seq() {
        let report = |pid: &str, load: &str, load_pid: &str| {
            Args::try_parse_from([
                "muster",
                SHELL_REPORT_SUBCOMMAND,
                "--pid",
                pid,
                "--load",
                load,
                "--load-pid",
                load_pid,
                "--seq",
                "7",
            ])
        };

        assert!(matches!(
            report("4242", "1793315867697448123", "5150").unwrap().command,
            Some(Command::ShellReport { pid, load, load_pid, seq })
                if pid.into_inner() == 4242
                    && load == LoadStart::new(1_793_315_867_697_448_123)
                    && load_pid.into_inner() == 5150
                    && seq == ReportSeq::new(7)
        ));
        for refused in ["1", "0", "-3", "shell"] {
            assert!(report(refused, "1", "5150").is_err(), "{refused}");
            assert!(report("4242", "1", refused).is_err(), "{refused}");
        }
        for refused in ["-1", "", "1.5", "load"] {
            assert!(report("4242", refused, "5150").is_err(), "{refused:?}");
        }
    }

    /// `shell-init` takes each hooked shell by name and lists them on a typo.
    #[test]
    fn shell_init_takes_each_hooked_shell() {
        for shell in HookShell::iter() {
            let args = Args::try_parse_from(["muster", "shell-init", &shell.to_string()]).unwrap();
            assert!(
                matches!(args.command, Some(Command::ShellInit { shell: parsed }) if parsed == shell)
            );
        }
        let Err(typo) = Args::try_parse_from(["muster", "shell-init", "tcsh"]) else {
            panic!("tcsh is refused");
        };
        let typo = typo.to_string();
        assert!(typo.contains("bash, zsh, fish"), "{typo}");
    }

    /// The hidden holder entry point takes its spec from its own `--spec`.
    #[test]
    fn the_holder_command_reads_its_spec_path() {
        let args =
            Args::try_parse_from(["muster", HOLDER_SUBCOMMAND, "--spec", "/run/holder.json"])
                .unwrap();

        assert!(matches!(
            args.command,
            Some(Command::Holder { spec }) if spec.as_os_str() == "/run/holder.json"
        ));
        assert_eq!(args.project, None);
        assert!(matches!(
            Args::try_parse_from(["muster", SERVICE_SUBCOMMAND]).map(|args| args.command),
            Ok(Some(Command::RunService))
        ));
    }
}
