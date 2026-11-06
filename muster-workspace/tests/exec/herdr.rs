//! `muster open` and MCP `process_open` against a real herdr: a server of
//! each test's own, its socket, config, state and home inside the test's
//! scratch. Run only with `MUSTER_HERDR_TEST=1` and herdr installed; each
//! test otherwise prints why it was skipped and passes.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Output,
};

use serde_json::Value;

use crate::common::{HerdrServer, McpSession, Scratch, entry, stack};

/// The stack file `import` reads.
const STACK_FILE: &str = "muster.yml";
/// The default detach key, `ctrl-\`, as text herdr types.
const DETACH: &str = "\u{1c}";
/// What every attach says once it shows its process.
const ATTACHED: &str = "attached to";
/// What the shown process prints, to tell its screen.
const WEB_OUTPUT: &str = "web-ran";
/// A process name built to break a typed shell line: quotes, command
/// substitutions that would create [`PWNED`], a semicolon and a leading
/// dash.
const HOSTILE_NAME: &str = "-x 'q' \"d\" $(touch pwned) `touch pwned`; touch pwned";
/// The file the hostile name would create if a shell ever ran it.
const PWNED: &str = "pwned";
/// What the hostile-named process prints.
const HOSTILE_OUTPUT: &str = "hostile-ran";
/// What `attach` says of a process that is not running.
const NOT_RUNNING: &str = "is not running";
/// A rc file line that holds a new pane's shell until [`GO`] exists in its
/// home, so a test can act between the split and the typed line.
const HELD_RC: &str = "while [ ! -e \"$HOME/go\" ]; do sleep 0.05; done\n";
/// The file that releases a held shell.
const GO: &str = "go";
/// The rc files bash reads, as an interactive shell and as a login one.
const BASH_RC_FILES: [&str; 2] = [".bashrc", ".bash_profile"];
/// A line typed into a pane once its attach has ended.
const STILL_HERE: &str = "echo still-$((40 + 2))";
/// What that line prints, and never echoes as typed.
const STILL_HERE_OUTPUT: &str = "still-42";
/// What the reading process prints once it reads a line.
const GOT: &str = "got-";
/// A line the read-only pane is typed, which must never arrive.
const IGNORED_LINE: &str = "q\r";
/// A line the interactive pane is typed.
const TYPED_LINE: &str = "z\r";

/// The project `folder` of `scratch`, holding `stack`, with every process
/// it autostarts running.
fn running_project(scratch: &Scratch, folder: &str, stack: &str) -> PathBuf {
    let dir = scratch.path().join(folder);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(STACK_FILE), stack).unwrap();
    succeeded(&scratch.muster(&["-p", dir.to_str().unwrap(), "import"]));
    succeeded(&scratch.muster(&["-p", dir.to_str().unwrap(), "up"]));
    dir
}

/// Fails the test, showing `output`, unless it succeeded.
fn succeeded(output: &Output) {
    assert!(
        output.status.success(),
        "status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Runs `muster open --json <args>` on `project` from `pane` of `herdr`,
/// and returns the pane it opened.
fn open(
    scratch: &Scratch,
    herdr: &HerdrServer,
    project: &Path,
    pane: &str,
    args: &[&str],
) -> String {
    let mut all = vec!["-p", project.to_str().unwrap(), "open", "--json"];
    all.extend_from_slice(args);
    let mut command = scratch.command(&all);
    herdr.place(&mut command, pane).current_dir(project);
    let output = command.output().unwrap();
    succeeded(&output);
    let view: Value = serde_json::from_slice(&output.stdout).unwrap();
    view["pane"].as_str().unwrap().to_string()
}

/// The `ps` row of `name` in `project`.
fn row(scratch: &Scratch, project: &Path, name: &str) -> Value {
    let output = scratch.muster(&["-p", project.to_str().unwrap(), "ps", "--json"]);
    succeeded(&output);
    let statuses: Value = serde_json::from_slice(&output.stdout).unwrap();
    statuses[0]["processes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|process| process["name"] == name)
        .cloned()
        .unwrap_or_else(|| panic!("no row {name}: {statuses}"))
}

/// Review Focus 4 and Decision 6: `open` splits a pane beside the caller
/// that attaches to the process and shows its screen; the detach key ends
/// the attach, which closes the pane, and the process keeps running.
#[test]
fn open_splits_a_pane_that_shows_the_process_and_closes_on_detach() {
    let scratch = Scratch::new();
    let Some(herdr) = HerdrServer::start(
        &scratch,
        "open_splits_a_pane_that_shows_the_process_and_closes_on_detach",
    ) else {
        return;
    };
    let project = running_project(
        &scratch,
        "shop",
        &stack(&[entry("web", "echo web-ran; sleep 600", true)]),
    );
    let root = herdr.workspace(&project);

    let pane = open(&scratch, &herdr, &project, &root, &["web"]);
    assert_ne!(pane, root);
    herdr.wait_output(&pane, &format!("{ATTACHED} web"));
    herdr.wait_output(&pane, WEB_OUTPUT);
    assert_eq!(herdr.pane(&root).unwrap()["focused"], false);
    assert_eq!(herdr.pane(&pane).unwrap()["focused"], true);

    herdr.send_text(&pane, DETACH);
    herdr.wait_closed(&pane);
    assert!(herdr.pane(&root).is_some());
    assert_eq!(row(&scratch, &project, "web")["state"], "running");
}

/// Review Focus 1: a name holding quotes, command substitutions, a
/// semicolon and a leading dash opens and attaches to that very process,
/// and nothing the name holds is ever run.
#[test]
fn a_hostile_name_opens_and_attaches_unharmed() {
    let scratch = Scratch::new();
    let Some(herdr) = HerdrServer::start(&scratch, "a_hostile_name_opens_and_attaches_unharmed")
    else {
        return;
    };
    let project = running_project(
        &scratch,
        "shop",
        &stack(&[entry(
            HOSTILE_NAME,
            &format!("echo {HOSTILE_OUTPUT}; sleep 600"),
            true,
        )]),
    );
    let root = herdr.workspace(&project);

    let pane = open(&scratch, &herdr, &project, &root, &["--", HOSTILE_NAME]);
    herdr.wait_output(&pane, ATTACHED);
    herdr.wait_output(&pane, HOSTILE_OUTPUT);
    herdr.send_text(&pane, DETACH);
    herdr.wait_closed(&pane);
    for folder in [project.as_path(), herdr.home(), scratch.root()] {
        assert!(!folder.join(PWNED).exists(), "{}", folder.display());
    }
}

/// Decisions 6 and 9: a process that stops between `open`'s locate and the
/// pane's attach leaves the pane open on attach's own message, its shell
/// still taking lines.
#[test]
fn a_refused_attach_leaves_the_pane_with_its_message() {
    let scratch = Scratch::new();
    let Some(herdr) = HerdrServer::start(
        &scratch,
        "a_refused_attach_leaves_the_pane_with_its_message",
    ) else {
        return;
    };
    let project = running_project(
        &scratch,
        "shop",
        &stack(&[entry("web", "echo web-ran; sleep 600", true)]),
    );
    let root = herdr.workspace(&project);
    for rc in BASH_RC_FILES {
        fs::write(herdr.home().join(rc), HELD_RC).unwrap();
    }

    let pane = open(&scratch, &herdr, &project, &root, &["web"]);
    succeeded(&scratch.muster(&["-p", project.to_str().unwrap(), "stop", "web"]));
    fs::write(herdr.home().join(GO), "").unwrap();
    herdr.wait_output(&pane, NOT_RUNNING);
    herdr.type_line(&pane, STILL_HERE);
    herdr.wait_output(&pane, STILL_HERE_OUTPUT);
    assert!(herdr.pane(&pane).is_some());
}

/// Decision 11: a read-only pane never passes typed keys to the process;
/// an interactive one does, and the process reads that line first.
#[test]
fn open_read_only_ignores_typed_keys() {
    let scratch = Scratch::new();
    let Some(herdr) = HerdrServer::start(&scratch, "open_read_only_ignores_typed_keys") else {
        return;
    };
    let project = running_project(
        &scratch,
        "shop",
        &stack(&[entry(
            "reader",
            "echo ready; read -r line; echo \"got-$line\"; sleep 600",
            true,
        )]),
    );
    let root = herdr.workspace(&project);

    let watching = open(&scratch, &herdr, &project, &root, &[
        "reader",
        "--read-only",
    ]);
    herdr.wait_output(&watching, &format!("{ATTACHED} reader"));
    herdr.wait_output(&watching, "ready");
    herdr.send_text(&watching, IGNORED_LINE);
    herdr.send_text(&watching, DETACH);
    herdr.wait_closed(&watching);

    let typing = open(&scratch, &herdr, &project, &root, &["reader"]);
    herdr.wait_output(&typing, &format!("{ATTACHED} reader"));
    herdr.send_text(&typing, TYPED_LINE);
    herdr.wait_output(&typing, &format!("{GOT}z"));
    assert!(
        !herdr.read(&typing).contains(&format!("{GOT}q")),
        "{}",
        herdr.read(&typing)
    );
    herdr.send_text(&typing, DETACH);
    herdr.wait_closed(&typing);
    assert_eq!(row(&scratch, &project, "reader")["state"], "running");
}

/// Review Focus 6 and Decision 12: an agent's `process_open` opens the pane
/// beside the agent's and leaves the focus on the agent's pane.
#[test]
fn mcp_process_open_splits_without_focus() {
    let mut scratch = Scratch::new();
    let Some(herdr) = HerdrServer::start(&scratch, "mcp_process_open_splits_without_focus") else {
        return;
    };
    let project = running_project(
        &scratch,
        "shop",
        &stack(&[entry("web", "echo web-ran; sleep 600", true)]),
    );
    let root = herdr.workspace(&project);

    let mut mcp = McpSession::start_placed(&mut scratch, &project, |command| {
        herdr.place(command, &root);
    });
    let opened = mcp.result("process_open", serde_json::json!({ "name": "web" }));
    assert_eq!(opened["isError"], false, "{opened}");
    let pane = opened["structuredContent"]["pane"]
        .as_str()
        .unwrap()
        .to_string();
    herdr.wait_output(&pane, &format!("{ATTACHED} web"));
    assert_eq!(herdr.pane(&root).unwrap()["focused"], true);
    assert_eq!(herdr.pane(&pane).unwrap()["focused"], false);

    herdr.send_text(&pane, DETACH);
    herdr.wait_closed(&pane);
}
