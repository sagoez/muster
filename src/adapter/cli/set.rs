//! `muster set`: changes to one process's stored definition. A running
//! process keeps running and takes the change at its next start.

use std::collections::BTreeSet;

use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::{
    command_words::command_string, error::CliError, invocation::Invocation, output::project_title,
    process_flags::env_name, report::Report, row::Row, row_kind::RowKind, set_args::SetArgs,
};
use crate::{
    adapter::client::{
        DefinitionView, ProjectIdentityView, WorkingDirPlace,
        process::{resolve_among, runs_now},
    },
    domain::{
        config::{ConfigError, FieldEdit, ProcessChange},
        coordination::Author,
        value::{CommandLine, Description},
    },
};

/// The command word of `muster set`.
const SET_WORD: &str = "set";
/// Opens the line naming the changed process.
const UPDATED: &str = "updated";
/// Follows the name of a process a set left as it was.
const NOTHING_CHANGED: &str = "nothing changed: it already holds these values";
/// Between a name and what changed.
const GAP: &str = "  ";
/// Between the changes listed.
const CHANGE_SEPARATOR: &str = ", ";
/// Names each kind of change in the summary.
const COMMAND_CHANGED: &str = "command";
const CWD_CHANGED: &str = "cwd";
const CWD_CLEARED: &str = "cwd cleared";
const DESCRIPTION_CHANGED: &str = "description";
const DESCRIPTION_CLEARED: &str = "description cleared";
const ENV_SET: &str = "env";
const ENV_UNSET: &str = "unset env";
const AUTOSTART_ON: &str = "autostart on";
const AUTOSTART_OFF: &str = "autostart off";
const RESTART_CHANGED: &str = "restart";
const STOP_SIGNAL_CHANGED: &str = "stop-signal";
const STOP_GRACE_CHANGED: &str = "stop-grace";
const READY_CHANGED: &str = "ready";
const READY_CLEARED: &str = "ready cleared";
const PORTS_CHANGED: &str = "ports";
const PORTS_CLEARED: &str = "ports cleared";
/// Between the ports a change names.
const PORT_SEPARATOR: &str = ", ";
/// Follows a running process's name in the restart hint.
const IS_RUNNING: &str = "is running;";
/// The command that applies a change to a running process.
const RESTART_COMMAND: &str = "muster restart";
/// Follows that command in the hint.
const APPLIES_IT: &str = "applies the change";

/// What `muster set` did, as `--json` shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct SetView {
    /// The process's project.
    #[getset(get = "pub")]
    project: ProjectIdentityView,
    /// The process as stored now.
    #[getset(get = "pub")]
    process: DefinitionView,
    /// Whether the stored definition changed; a set to the values already
    /// stored succeeds and changes nothing.
    #[getset(get_copy = "pub")]
    changed: bool,
    /// Whether a running service reports it running, so the change waits
    /// for its next start.
    #[getset(get_copy = "pub")]
    running: bool,
    /// Whether a running process only takes the change at its next start.
    #[serde(skip)]
    #[getset(get_copy = "pub")]
    needs_restart: bool,
    /// What changed, in words.
    #[serde(skip)]
    #[getset(get = "pub")]
    changes: Vec<String>,
}

/// Applies the changes `args` asks for to one process of the invocation's
/// project. A running service is asked whether the process runs, never
/// started for it; one that cannot be asked counts as not running, since the
/// change is stored by then.
///
/// # Errors
/// Returns [`CliError::NothingToChange`] when no change was given, the
/// [`CliError`] of an unknown process or an unusable value, or a stored-state
/// failure.
pub fn set(invocation: &Invocation<'_>, args: SetArgs) -> Result<SetView, CliError> {
    if asks_nothing(&args) {
        return Err(CliError::NothingToChange);
    }
    let command = match args.command() {
        Some(raw) => Some(
            CommandLine::try_new(raw.as_str()).map_err(|error| CliError::command(raw, &error))?,
        ),
        None if args.words().is_empty() => None,
        None => Some(command_string(args.words())?),
    };
    let store = invocation.store()?;
    let project = invocation.editing_project(store.as_ref())?;
    let root = project.root();
    let working_dir = match (args.cwd().clone(), args.no_cwd()) {
        (Some(dir), _) => match WorkingDirPlace::of(root, &invocation.in_cwd(&dir)?)?.stored() {
            Some(dir) => FieldEdit::Set(dir),
            None => FieldEdit::Clear,
        },
        (None, true) => FieldEdit::Clear,
        (None, false) => FieldEdit::Keep,
    };
    let description = match (args.description().clone(), args.no_description()) {
        (Some(text), _) => FieldEdit::Set(
            Description::try_new(text.as_str())
                .map_err(|error| CliError::description(&text, &error))?,
        ),
        (None, true) => FieldEdit::Clear,
        (None, false) => FieldEdit::Keep,
    };
    let change = ProcessChange::builder()
        .command(command)
        .working_dir(working_dir)
        .description(description)
        .set_env(args.flags().environment()?)
        .unset_env(
            args.unset_env()
                .iter()
                .map(|raw| env_name(raw))
                .collect::<Result<_, _>>()?,
        )
        .autostart(args.flags().autostart())
        .restart(args.flags().restart())
        .stop_signal(args.flags().stop_signal())
        .stop_grace(args.flags().stop_grace())
        .ready(match (args.flags().ready()?, args.no_ready()) {
            (Some(check), _) => FieldEdit::Set(check),
            (None, true) => FieldEdit::Clear,
            (None, false) => FieldEdit::Keep,
        })
        .ports(match (args.flags().ports(), args.no_ports()) {
            (Some(ports), _) => Some(ports),
            (None, true) => Some(BTreeSet::new()),
            (None, false) => None,
        })
        .build();
    let stack = store
        .stack(root)?
        .ok_or_else(|| ConfigError::UnknownProject {
            root: root.as_ref().clone(),
        })?;
    let name = resolve_among(stack.names(), &project, args.name())?;
    let applied =
        invocation
            .editor(store.as_ref())
            .change(root, &name, &change, &Author::human())?;
    let running = runs_now(invocation.service(), root, applied.spec().name());
    Ok(SetView::builder()
        .project(ProjectIdentityView::from(&project))
        .process(DefinitionView::from(applied.spec()))
        .changed(applied.changed())
        .running(running)
        .needs_restart(applied.changed() && change.needs_restart())
        .changes(change_words(&change))
        .build())
}

/// Whether `args` asks for no change at all, which is refused before any
/// state is read.
fn asks_nothing(args: &SetArgs) -> bool {
    args.command().is_none()
        && args.words().is_empty()
        && args.cwd().is_none()
        && !args.no_cwd()
        && args.description().is_none()
        && !args.no_description()
        && args.unset_env().is_empty()
        && !args.no_ready()
        && !args.no_ports()
        && args.flags().is_empty()
}

/// What `change` changes, in words: `restart on-failure`, `env PORT`.
fn change_words(change: &ProcessChange) -> Vec<String> {
    let mut words = Vec::new();
    if change.command().is_some() {
        words.push(COMMAND_CHANGED.to_string());
    }
    match change.working_dir() {
        FieldEdit::Set(_) => words.push(CWD_CHANGED.to_string()),
        FieldEdit::Clear => words.push(CWD_CLEARED.to_string()),
        FieldEdit::Keep => {},
    }
    match change.description() {
        FieldEdit::Set(_) => words.push(DESCRIPTION_CHANGED.to_string()),
        FieldEdit::Clear => words.push(DESCRIPTION_CLEARED.to_string()),
        FieldEdit::Keep => {},
    }
    words.extend(
        change
            .set_env()
            .iter()
            .map(|(name, _)| format!("{ENV_SET} {name}")),
    );
    words.extend(
        change
            .unset_env()
            .iter()
            .map(|name| format!("{ENV_UNSET} {name}")),
    );
    if let Some(autostart) = change.autostart() {
        words.push(
            if *autostart {
                AUTOSTART_ON
            } else {
                AUTOSTART_OFF
            }
            .to_string(),
        );
    }
    if let Some(restart) = change.restart() {
        words.push(format!("{RESTART_CHANGED} {restart}"));
    }
    if let Some(signal) = change.stop_signal() {
        words.push(format!("{STOP_SIGNAL_CHANGED} {signal}"));
    }
    if let Some(grace) = change.stop_grace() {
        words.push(format!("{STOP_GRACE_CHANGED} {grace}"));
    }
    match change.ready() {
        FieldEdit::Set(_) => words.push(READY_CHANGED.to_string()),
        FieldEdit::Clear => words.push(READY_CLEARED.to_string()),
        FieldEdit::Keep => {},
    }
    match change.ports() {
        Some(ports) if ports.is_empty() => words.push(PORTS_CLEARED.to_string()),
        Some(ports) => words.push(format!(
            "{PORTS_CHANGED} {}",
            ports
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(PORT_SEPARATOR)
        )),
        None => {},
    }
    words
}

/// The human report of `view`: what changed, and how a running process
/// takes a change that needs a restart, or that nothing changed.
pub fn set_rows(view: &SetView) -> Report {
    let name = view.process().name();
    let title = project_title(SET_WORD, view.project().name());
    if !view.changed() {
        return Report::new(title, vec![Row::unlabeled(
            RowKind::Ok,
            format!("{name}{GAP}{NOTHING_CHANGED}"),
        )]);
    }
    let mut rows = vec![Row::unlabeled(
        RowKind::Ok,
        format!(
            "{UPDATED} {name}{GAP}{}",
            view.changes().join(CHANGE_SEPARATOR)
        ),
    )];
    if view.running() && view.needs_restart() {
        rows.push(Row::unlabeled(
            RowKind::Dim,
            format!(
                "{name} {IS_RUNNING} `{RESTART_COMMAND} {}` {APPLIES_IT}",
                name.shell_quoted()
            ),
        ));
    }
    Report::new(title, rows)
}

#[cfg(test)]
mod tests {
    use std::{
        fs, iter,
        path::{Path, PathBuf},
        time::Duration,
    };

    use clap::{Parser, error::ErrorKind};
    use tempfile::TempDir;

    use super::{
        super::{add::add, add_args::AddArgs, args::Args, command::Command, fake_ports::FakePorts},
        *,
    };
    use crate::{
        adapter::{cli::report::plain, store::test_support::fresh_store},
        domain::{
            config::{Environment, ProcessSpec},
            port::StackReader,
            process::{RestartPolicy, StopPolicy, StopSignal},
            value::{
                CommandLine, EnvName, EnvValue, LocalPort, ProcessName, ProjectName, ProjectRoot,
                StopGrace,
            },
        },
    };

    /// `muster` run with `words`, parsed.
    fn parse(words: &[&str]) -> Result<Command, clap::Error> {
        Args::try_parse_from(iter::once("muster").chain(words.iter().copied()))
            .map(|args| args.into_command().expect("a command was given"))
    }

    /// `muster add <words>` parsed.
    fn add_args(words: &[&str]) -> AddArgs {
        match parse(&[&["add"], words].concat()).unwrap() {
            Command::Add(args) => args,
            _ => panic!("add parses as add"),
        }
    }

    /// `muster set <words>` parsed.
    fn set_args(words: &[&str]) -> SetArgs {
        match parse(&[&["set"], words].concat()).unwrap() {
            Command::Set(args) => args,
            _ => panic!("set parses as set"),
        }
    }

    /// The ports `raw` names.
    fn ports(raw: &[u16]) -> BTreeSet<LocalPort> {
        raw.iter()
            .map(|port| LocalPort::try_new(*port).unwrap())
            .collect()
    }

    /// Ports over a fresh database, the folder of a project `shop` in it,
    /// and the scratch dir holding both.
    fn shop() -> (FakePorts, PathBuf, TempDir) {
        let (store, scratch) = fresh_store();
        let dir = scratch.path().join("shop");
        fs::create_dir_all(&dir).unwrap();
        (FakePorts::new(store), dir, scratch)
    }

    /// The ports stored for `web` in the project at `dir`.
    fn stored_ports(fakes: &FakePorts, dir: &Path) -> BTreeSet<LocalPort> {
        let root = ProjectRoot::try_new(dir.to_path_buf()).unwrap();
        let stack = fakes.store().stack(&root).unwrap().unwrap();
        stack
            .process(&ProcessName::try_new("web").unwrap())
            .unwrap()
            .ports()
            .clone()
    }

    /// `add --port` stores each port once, and `set --port` replaces the
    /// whole set, each through the editor into the database.
    #[test]
    fn add_and_set_take_ports() {
        let (fakes, dir, _scratch) = shop();
        let invocation = Invocation::new(Some(dir.to_str().unwrap().to_string()), &fakes);

        let added = add(
            &invocation,
            add_args(&[
                "web",
                "--port",
                "5173",
                "--port",
                "3000",
                "--port",
                "5173",
                "--",
                "npm run dev",
            ]),
        )
        .unwrap();
        assert_eq!(added.process().ports(), &[3000, 5173]);
        assert_eq!(stored_ports(&fakes, &dir), ports(&[3000, 5173]));

        let changed = set(&invocation, set_args(&["web", "--port", "8080"])).unwrap();
        assert!(changed.changed());
        assert!(!changed.needs_restart(), "a ports change restarts nothing");
        assert_eq!(changed.process().ports(), &[8080]);
        assert_eq!(changed.changes(), &["ports 8080"]);
        assert_eq!(stored_ports(&fakes, &dir), ports(&[8080]));

        let plain_add = add(&invocation, add_args(&["api", "--", "bin/api"])).unwrap();
        assert!(plain_add.process().ports().is_empty());
    }

    /// `--no-ports` clears the set, and cannot be given with `--port`.
    #[test]
    fn no_ports_clears_and_conflicts_with_port() {
        let (fakes, dir, _scratch) = shop();
        let invocation = Invocation::new(Some(dir.to_str().unwrap().to_string()), &fakes);
        add(
            &invocation,
            add_args(&["web", "--port", "3000", "--", "npm run dev"]),
        )
        .unwrap();

        let cleared = set(&invocation, set_args(&["web", "--no-ports"])).unwrap();
        assert!(cleared.changed());
        assert!(cleared.process().ports().is_empty());
        assert_eq!(cleared.changes(), &["ports cleared"]);
        assert!(stored_ports(&fakes, &dir).is_empty());

        let again = set(&invocation, set_args(&["web", "--no-ports"])).unwrap();
        assert!(!again.changed(), "clearing no ports changes nothing");

        let both = parse(&["set", "web", "--port", "3000", "--no-ports"]).err();
        assert_eq!(
            both.map(|error| error.kind()),
            Some(ErrorKind::ArgumentConflict)
        );
    }

    /// A port outside 1 to 65535 is a usage error (exit 2) naming the form
    /// a port is written in, for `add` and `set` alike.
    #[test]
    fn a_zero_port_is_refused_by_the_parser_naming_the_form() {
        for words in [
            &["add", "web", "--port", "0", "--", "x"][..],
            &["set", "web", "--port", "0"],
            &["set", "web", "--port", "65536"],
        ] {
            let error = parse(words).err().expect("the port is refused");
            assert_eq!(error.kind(), ErrorKind::ValueValidation, "{words:?}");
            assert_eq!(error.exit_code(), 2, "{words:?}");
            assert!(
                error.to_string().contains("a port from 1 to 65535"),
                "{error}"
            );
        }
    }

    /// A ports change is summarised by its ports, or as cleared.
    #[test]
    fn set_summarises_a_ports_change() {
        let replaced = ProcessChange::builder()
            .ports(Some(ports(&[5173, 3000])))
            .build();
        assert_eq!(
            plain(&set_rows(&running_view(&replaced))),
            "updated api  ports 3000, 5173"
        );
        let cleared = ProcessChange::builder()
            .ports(Some(BTreeSet::new()))
            .build();
        assert_eq!(
            plain(&set_rows(&running_view(&cleared))),
            "updated api  ports cleared"
        );
    }

    /// The view of a change to the running `api`, `needs_restart` or not.
    fn running_view(change: &ProcessChange) -> SetView {
        let spec = ProcessSpec::builder()
            .name(ProcessName::try_new("api").unwrap())
            .command(CommandLine::try_new("bin/api").unwrap())
            .environment(Environment::empty())
            .autostart(true)
            .restart(RestartPolicy::Never)
            .stop(
                StopPolicy::builder()
                    .signal(StopSignal::Terminate)
                    .grace_period(StopGrace::try_new(Duration::from_secs(5)).unwrap())
                    .build(),
            )
            .build();
        SetView::builder()
            .project(
                ProjectIdentityView::builder()
                    .name(ProjectName::try_new("shop").unwrap())
                    .root(PathBuf::from("/p"))
                    .build(),
            )
            .process(DefinitionView::from(&spec))
            .changed(true)
            .running(true)
            .needs_restart(change.needs_restart())
            .changes(change_words(change))
            .build()
    }

    /// A set that changed nothing says so, with no restart hint, and its
    /// JSON says `changed: false`.
    #[test]
    fn a_set_that_changed_nothing_says_so() {
        let command = ProcessChange::builder()
            .command(Some(CommandLine::try_new("bin/api").unwrap()))
            .build();
        let view = SetView::builder()
            .project(running_view(&command).project().clone())
            .process(running_view(&command).process().clone())
            .changed(false)
            .running(true)
            .needs_restart(false)
            .changes(change_words(&command))
            .build();

        assert_eq!(plain(&set_rows(&view)), format!("api  {NOTHING_CHANGED}"));
        assert_eq!(serde_json::to_value(&view).unwrap()["changed"], false);
    }

    /// A running process is told to restart only for a change it reads at
    /// launch, never for its description or autostart.
    #[test]
    fn the_restart_hint_follows_what_changed() {
        let quiet = ProcessChange::builder()
            .description(FieldEdit::Clear)
            .autostart(Some(false))
            .build();
        assert_eq!(
            plain(&set_rows(&running_view(&quiet))),
            "updated api  description cleared, autostart off"
        );

        let command = ProcessChange::builder()
            .command(Some(CommandLine::try_new("bin/api --v2").unwrap()))
            .build();
        assert!(
            plain(&set_rows(&running_view(&command))).contains("`muster restart 'api'`"),
            "{}",
            plain(&set_rows(&running_view(&command)))
        );
    }

    /// Each kind of change is named in the summary, variables by name only.
    #[test]
    fn a_change_is_summarised_in_words() {
        let change = ProcessChange::builder()
            .working_dir(FieldEdit::Clear)
            .set_env(vec![(
                EnvName::try_new("PORT").unwrap(),
                EnvValue::try_new("secret").unwrap(),
            )])
            .restart(Some(RestartPolicy::OnFailure))
            .stop_grace(Some(StopGrace::try_new(Duration::from_secs(10)).unwrap()))
            .ready(FieldEdit::Clear)
            .build();

        assert_eq!(change_words(&change), vec![
            "cwd cleared",
            "env PORT",
            "restart on-failure",
            "stop-grace 10s",
            "ready cleared"
        ]);
    }
}
