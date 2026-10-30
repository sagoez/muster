//! `muster mcp add` and `muster mcp remove`: muster's MCP server entry
//! written into, or taken out of, agents' own configs.

use strum::IntoEnumIterator;

use super::{
    error::CliError,
    invocation::Invocation,
    mcp_edit_view::{McpConfigEdit, McpEditView},
    mcp_entry_args::McpEntryArgs,
    presenter::Presenter,
    report::Report,
    row::Row,
    row_kind::RowKind,
};
use crate::adapter::agent_config::{
    Agent, AgentConfigError, ConfigTarget, EditOutcome, EntryChange, McpScope,
};

/// The title of `muster mcp add`.
const ADD_TITLE: &str = "muster mcp add";
/// The title of `muster mcp remove`.
const REMOVE_TITLE: &str = "muster mcp remove";
/// What is said when no agent was chosen.
const NONE_CHOSEN: &str = "no agent chosen; nothing changed";
/// How a run that wrote a config closes.
const RESTART_HINT: &str = "start a new agent session to load the change";
/// How a run that wrote nothing closes.
const NOTHING_CHANGED: &str = "nothing to change";

/// Makes `change` to the entry `args` names in the configs of the agents
/// `args` names, or that the person picks at a terminal. Every config is
/// read and checked before any is written, so a refusal leaves them all as
/// they were.
///
/// # Errors
/// Returns [`CliError::CannotAsk`] without `--client` where nobody can be
/// asked, the [`CliError`] of a project a scope needs and cannot find, or
/// [`CliError::AgentConfig`] for a scope an agent lacks, a file muster
/// cannot edit safely, or an entry it must not touch.
pub fn mcp_entries(
    invocation: &Invocation<'_>,
    presenter: Presenter,
    change: EntryChange,
    args: &McpEntryArgs,
) -> Result<McpEditView, CliError> {
    let registry = invocation.ports().mcp_registry();
    let agents = if args.client().is_empty() {
        presenter.can_choose_agents()?;
        let offered: Vec<Agent> = Agent::iter()
            .filter(|agent| args.scope().is_none_or(|scope| agent.supports(scope)))
            .collect();
        presenter.choose_agents(change, &offered, &registry.present())?
    } else {
        distinct(args.client())
    };
    let scoped = agents
        .iter()
        .map(|agent| {
            let scope = args.scope().unwrap_or_else(|| agent.default_scope());
            if agent.supports(scope) {
                Ok((*agent, scope))
            } else {
                Err(AgentConfigError::UnsupportedScope {
                    agent: *agent,
                    scope,
                })
            }
        })
        .collect::<Result<Vec<(Agent, McpScope)>, _>>()?;
    let root = if scoped.iter().any(|(_, scope)| scope.needs_project()) {
        let store = invocation.store()?;
        Some(invocation.project(store.as_ref())?.root().clone())
    } else {
        None
    };
    let planned = scoped
        .into_iter()
        .map(|(agent, scope)| {
            let target = ConfigTarget::builder()
                .agent(agent)
                .scope(scope)
                .root(root.clone())
                .build();
            registry.plan(&target, args.name(), change, args.force())
        })
        .collect::<Result<Vec<_>, _>>()?;
    for edit in &planned {
        edit.commit()?;
    }
    Ok(McpEditView::builder()
        .change(change)
        .edits(
            planned
                .iter()
                .map(|edit| McpConfigEdit::of(edit, args.name()))
                .collect(),
        )
        .build())
}

/// `agents` with each one kept once, in the order first given.
fn distinct(agents: &[Agent]) -> Vec<Agent> {
    let mut kept = Vec::with_capacity(agents.len());
    for agent in agents {
        if !kept.contains(agent) {
            kept.push(*agent);
        }
    }
    kept
}

/// `view` as a person reads it: a line per config saying what was done
/// where, then what the agent does with the entry.
pub fn mcp_rows(view: &McpEditView) -> Report {
    let title = match view.change() {
        EntryChange::Add => ADD_TITLE,
        EntryChange::Remove => REMOVE_TITLE,
    };
    let mut rows = Vec::new();
    for edit in view.edits() {
        rows.push(Row::labeled(
            RowKind::Ok,
            edit.agent().title(),
            edit_line(edit),
        ));
        if let Some(note) = edit.note() {
            rows.push(Row::unlabeled(RowKind::Hint, note));
        }
    }
    if rows.is_empty() {
        rows.push(Row::unlabeled(RowKind::Dim, NONE_CHOSEN));
    }
    let summary = if view.edits().iter().any(|edit| edit.outcome().writes()) {
        RESTART_HINT
    } else {
        NOTHING_CHANGED
    };
    Report::new(title, rows).with_summary(summary)
}

/// What was done to `edit`'s config, and where.
fn edit_line(edit: &McpConfigEdit) -> String {
    let name = edit.name();
    let path = edit.path().display();
    let scope = edit.scope();
    match edit.outcome() {
        EditOutcome::Added => format!("added {name} to {path} ({scope} scope)"),
        EditOutcome::AlreadyAdded => format!("{name} is already in {path} ({scope} scope)"),
        EditOutcome::Replaced => format!("replaced {name} in {path} ({scope} scope)"),
        EditOutcome::Removed => format!("removed {name} from {path} ({scope} scope)"),
        EditOutcome::Absent => format!("no {name} in {path} ({scope} scope)"),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs, iter,
        path::{Path, PathBuf},
    };

    use clap::Parser;
    use serde_json::{Value, json};
    use tempfile::TempDir;

    use super::*;
    use crate::adapter::{
        cli::{
            add::add,
            args::Args,
            command::Command,
            fake_ports::{FAKE_MUSTER, FakePorts},
            mcp_command::McpCommand,
            presenter::CHOOSE_AGENTS_ARGUMENT,
            report::plain,
        },
        client::FailureKind,
        store::test_support::fresh_store,
    };

    /// `muster <words>` parsed into its command.
    fn parse(words: &[&str]) -> Command {
        Args::try_parse_from(iter::once("muster").chain(words.iter().copied()))
            .unwrap()
            .into_command()
            .expect("a command was given")
    }

    /// `muster mcp add <words>` or `remove` parsed into its arguments.
    fn entry_args(change: EntryChange, words: &[&str]) -> McpEntryArgs {
        let verb = change.to_string();
        match parse(&[&["mcp", verb.as_str()], words].concat()) {
            Command::Mcp {
                command: Some(McpCommand::Add(args) | McpCommand::Remove(args)),
                ..
            } => args,
            _ => panic!("mcp {verb} parses as such"),
        }
    }

    /// Ports over a fresh database, a project `shop` registered in it, and
    /// the scratch dir holding both.
    fn shop() -> (FakePorts, PathBuf, TempDir) {
        let (store, scratch) = fresh_store();
        let dir = scratch.path().join("shop");
        fs::create_dir_all(&dir).unwrap();
        let fakes = FakePorts::new(store);
        let invocation = Invocation::new(Some(dir.to_str().unwrap().to_owned()), &fakes);
        let Command::Add(args) = parse(&["add", "web", "--", "npm run dev"]) else {
            panic!("add parses as add");
        };
        add(&invocation, args).unwrap();
        (fakes, dir.canonicalize().unwrap(), scratch)
    }

    /// Runs `muster mcp <change> <words>` on `fakes` in the project at `dir`.
    fn run(
        fakes: &FakePorts,
        dir: &Path,
        change: EntryChange,
        words: &[&str],
    ) -> Result<McpEditView, CliError> {
        let invocation = Invocation::new(Some(dir.to_str().unwrap().to_owned()), fakes);
        mcp_entries(
            &invocation,
            Presenter::Plain,
            change,
            &entry_args(change, words),
        )
    }

    /// The JSON file at `path`.
    fn json_at(path: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    /// muster's entry as Claude Code reads it.
    fn ours() -> Value {
        json!({"command": FAKE_MUSTER, "args": ["mcp"]})
    }

    /// A name no agent takes as a key is refused with the form it must
    /// take, and `--as` belongs to serving, not to a subcommand.
    #[test]
    fn a_bad_name_or_a_serving_flag_is_refused_on_parsing() {
        let Err(bad) = Args::try_parse_from(["muster", "mcp", "add", "--name", "a b"]) else {
            panic!("a spaced name parses");
        };
        assert!(
            bad.to_string().contains("use 1 to 64 ASCII letters"),
            "{bad}"
        );
        assert!(Args::try_parse_from(["muster", "mcp", "--as", "x", "add"]).is_err());
        assert!(Args::try_parse_from(["muster", "mcp", "--print-config", "remove"]).is_err());
    }

    /// Without --client and nobody to ask, the command names the flag and
    /// writes nothing.
    #[test]
    fn without_a_client_and_a_terminal_it_asks_for_the_flag() {
        let (fakes, dir, _scratch) = shop();

        let refused = run(&fakes, &dir, EntryChange::Add, &[]).unwrap_err();
        assert!(matches!(refused, CliError::CannotAsk {
            argument: CHOOSE_AGENTS_ARGUMENT,
            ..
        }));
        assert_eq!(fs::read_dir(fakes.agent_home()).unwrap().count(), 0);
    }

    /// Claude Code's entry goes to the project's place in `.claude.json` by
    /// default; a second add reports it already there, and remove takes
    /// only it out.
    #[test]
    fn claude_defaults_to_the_local_scope_and_is_idempotent() {
        let (fakes, dir, _scratch) = shop();
        let claude = fakes.agent_home().join(".claude.json");
        fs::write(&claude, "{\"numStartups\": 2, \"mcpServers\": {\"other\": {\"command\": \"x\", \"args\": []}}}\n").unwrap();
        let project = dir.display().to_string();

        let added = run(&fakes, &dir, EntryChange::Add, &["--client", "claude"]).unwrap();
        assert_eq!(added.edits()[0].scope(), McpScope::Local);
        assert_eq!(added.edits()[0].outcome(), EditOutcome::Added);
        assert_eq!(
            json_at(&claude)["projects"][&project]["mcpServers"]["muster"],
            ours()
        );
        assert_eq!(json_at(&claude)["numStartups"], 2);

        let again = run(&fakes, &dir, EntryChange::Add, &["--client", "claude"]).unwrap();
        assert_eq!(again.edits()[0].outcome(), EditOutcome::AlreadyAdded);
        assert!(plain(&mcp_rows(&again)).contains("muster is already in"));

        let removed = run(&fakes, &dir, EntryChange::Remove, &["--client", "claude"]).unwrap();
        assert_eq!(removed.edits()[0].outcome(), EditOutcome::Removed);
        let after = json_at(&claude);
        assert!(
            after["projects"][&project]["mcpServers"]
                .get("muster")
                .is_none()
        );
        assert_eq!(after["mcpServers"]["other"]["command"], "x");
    }

    /// A project scope Zed lacks, or a local one Codex lacks, is refused
    /// before any config is touched.
    #[test]
    fn a_scope_the_agent_lacks_is_refused_before_writing() {
        let (fakes, dir, _scratch) = shop();

        for words in [
            &["--client", "claude,zed", "--scope", "project"][..],
            &["--client", "codex", "--scope", "local"][..],
        ] {
            let refused = run(&fakes, &dir, EntryChange::Add, words).unwrap_err();
            assert!(
                matches!(
                    refused,
                    CliError::AgentConfig(AgentConfigError::UnsupportedScope { .. })
                ),
                "{words:?}: {refused}"
            );
        }
        assert!(
            !dir.join(".mcp.json").exists(),
            "claude's file waits for zed's refusal"
        );
    }

    /// One config muster cannot edit stops the run before any is written,
    /// and a different entry under the name needs --force.
    #[test]
    fn every_config_is_checked_before_any_is_written() {
        let (fakes, dir, _scratch) = shop();
        let zed = fakes.agent_home().join(".config/zed/settings.json");
        fs::create_dir_all(zed.parent().unwrap()).unwrap();
        fs::write(&zed, "{ \"theme\": ").unwrap();

        let refused = run(&fakes, &dir, EntryChange::Add, &["--client", "codex,zed"]).unwrap_err();
        assert!(matches!(
            refused,
            CliError::AgentConfig(AgentConfigError::Unparseable { .. })
        ));
        assert!(!fakes.agent_home().join(".codex").exists());

        fs::write(
            &zed,
            "{\"context_servers\": {\"muster\": {\"command\": \"/old\", \"args\": []}}}",
        )
        .unwrap();
        let taken = run(&fakes, &dir, EntryChange::Add, &["--client", "zed"]).unwrap_err();
        assert_eq!(taken.kind(), FailureKind::Conflict);
        let forced = run(&fakes, &dir, EntryChange::Add, &[
            "--client", "zed", "--force",
        ])
        .unwrap();
        assert_eq!(forced.edits()[0].outcome(), EditOutcome::Replaced);
        assert_eq!(
            json_at(&zed)["context_servers"]["muster"]["command"],
            FAKE_MUSTER
        );
    }

    /// A project scope outside every project fails as such; the user scope
    /// needs no project.
    #[test]
    fn a_project_scope_needs_a_project() {
        let (store, scratch) = fresh_store();
        let fakes = FakePorts::new(store);
        let elsewhere = scratch.path();

        let refused =
            run(&fakes, elsewhere, EntryChange::Add, &["--client", "claude"]).unwrap_err();
        assert!(
            matches!(
                refused,
                CliError::UnknownProject(_) | CliError::NotAProject { .. }
            ),
            "{refused}"
        );
        let user = run(&fakes, elsewhere, EntryChange::Add, &[
            "--client", "claude", "--scope", "user", "--name", "mine",
        ])
        .unwrap();
        assert_eq!(user.edits()[0].outcome(), EditOutcome::Added);
        assert_eq!(
            json_at(&fakes.agent_home().join(".claude.json"))["mcpServers"]["mine"],
            ours()
        );
    }

    /// Codex's project config is written in the project, with the note that
    /// Codex reads it only once the project is trusted.
    #[test]
    fn codex_project_scope_carries_its_trust_note() {
        let (fakes, dir, _scratch) = shop();

        let added = run(&fakes, &dir, EntryChange::Add, &[
            "--client", "codex", "--scope", "project", "--json",
        ])
        .unwrap();
        let shown = plain(&mcp_rows(&added));
        assert!(shown.contains("trusted"), "{shown}");
        assert!(
            fs::read_to_string(dir.join(".codex/config.toml"))
                .unwrap()
                .contains("[mcp_servers.muster]")
        );
        let json = serde_json::to_value(&added).unwrap();
        assert_eq!(json["change"], "add");
        assert_eq!(json["edits"][0]["agent"], "codex");
        assert_eq!(json["edits"][0]["scope"], "project");
        assert_eq!(json["edits"][0]["outcome"], "added");
    }
}
