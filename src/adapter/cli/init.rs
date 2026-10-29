//! `muster init`: processes found in the files a folder already has, offered
//! to add. Detection only reads files; what is chosen is added in one
//! transaction, registering the folder.

use getset::Getters;
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::{
    error::CliError,
    invocation::stack_editor,
    report::{Report, Row, RowKind},
};
use crate::{
    adapter::{
        detect::{Candidate, DetectSource, detect},
        store::SqliteStore,
    },
    domain::{
        config::{ImportMode, ProcessDraft, ProcessSpec},
        port::{SettingsStore, StackStore},
        value::{ProcessName, ProjectRoot},
    },
};

/// Title of the `init` reports.
pub const INIT_TITLE: &str = "muster init";
/// Marks a suggested candidate in the plain list.
const CHOSEN_MARK: &str = "[x]";
/// Marks a candidate that is not suggested.
const UNCHOSEN_MARK: &str = "[ ]";
/// Between the columns of a candidate's line.
const COLUMN_GAP: &str = "  ";
/// How the plain list ends.
const YES_HINT: &str = "`muster init --yes` adds the suggested ones";
/// Said when nothing was found.
const NOTHING_FOUND: &str =
    "nothing to suggest here; `muster add <name> -- <command>` adds a process";
/// Opens the step naming where candidates were found.
const FOUND_IN: &str = "found processes in";
/// Between the sources named.
const SOURCE_SEPARATOR: &str = ", ";
/// Marks an added process.
const ADDED_MARK: &str = "+";
/// Opens the outro of an interactive `init`.
const ADDED: &str = "added";
/// One process, counted.
const PROCESS: &str = "process";
/// Several processes, counted.
const PROCESSES: &str = "processes";
/// How what was added starts, when it autostarts.
const UP_HINT: &str = "`muster up` starts them";
/// How what was added starts, when it does not.
const START_HINT: &str = "`muster start <name>` starts one";
/// Said when the person chose nothing.
pub const NOTHING_ADDED: &str = "nothing added";
/// The question whether `muster up` starts what is added.
pub const AUTOSTART_QUESTION: &str = "Start them with `muster up`?";
/// What the plain layout says it could not ask.
pub const AUTOSTART_ASKED: &str = "whether `muster up` starts them";

/// What `muster init` offered and added, as `--json` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct InitView {
    /// Every process offered.
    candidates: Vec<Candidate>,
    /// The ones added.
    added: Vec<ProcessName>,
}

/// The candidates found under `root`, leaving out names the project
/// already has.
///
/// # Errors
/// Returns the [`CliError`] of a stack that cannot be read.
pub fn offered(store: &SqliteStore, root: &ProjectRoot) -> Result<Vec<Candidate>, CliError> {
    let stack = store.stack(root)?;
    Ok(detect(root)
        .into_iter()
        .filter(|candidate| {
            stack
                .as_ref()
                .is_none_or(|stack| stack.process(candidate.name()).is_none())
        })
        .collect())
}

/// The suggested ones of `candidates`.
pub fn suggested(candidates: &[Candidate]) -> Vec<Candidate> {
    candidates
        .iter()
        .filter(|candidate| candidate.suggested())
        .cloned()
        .collect()
}

/// Adds `chosen` to the project at `root` in one transaction, registering
/// it when new, `autostart` as asked or the default when not asked.
///
/// # Errors
/// Returns the [`CliError`] of a name the project took meanwhile or a store
/// that cannot be written.
pub fn add_chosen(
    store: &SqliteStore,
    root: &ProjectRoot,
    chosen: &[Candidate],
    autostart: Option<bool>,
) -> Result<Vec<ProcessName>, CliError> {
    if chosen.is_empty() {
        return Ok(Vec::new());
    }
    let defaults = store.load()?.process_defaults();
    let specs: Vec<ProcessSpec> = chosen
        .iter()
        .map(|candidate| {
            ProcessDraft::builder()
                .name(candidate.name().clone())
                .command(candidate.command().clone())
                .autostart(autostart)
                .build()
                .complete(&defaults)
        })
        .collect();
    let (_, plan) = stack_editor(store).import(root, &specs, ImportMode::Strict)?;
    Ok(plan.added().clone())
}

/// The plain list of `candidates`, the suggested ones checked, and how to
/// add them.
pub fn candidate_rows(candidates: &[Candidate]) -> Report {
    if candidates.is_empty() {
        return nothing_found();
    }
    let mut rows: Vec<Row> = candidates
        .iter()
        .map(|candidate| {
            let mark = if candidate.suggested() {
                CHOSEN_MARK
            } else {
                UNCHOSEN_MARK
            };
            Row::unlabeled(
                RowKind::Plain,
                format!(
                    "{mark} {}{COLUMN_GAP}{}{COLUMN_GAP}{}",
                    candidate.name(),
                    candidate.command(),
                    candidate.source()
                ),
            )
        })
        .collect();
    rows.push(Row::unlabeled(RowKind::Dim, YES_HINT));
    Report::new(INIT_TITLE, rows)
}

/// The report when nothing was found to offer.
pub fn nothing_found() -> Report {
    Report::new(INIT_TITLE, vec![Row::unlabeled(
        RowKind::Hint,
        NOTHING_FOUND,
    )])
}

/// The step naming the sources `candidates` came from.
pub fn found_in(candidates: &[Candidate]) -> String {
    let mut sources: Vec<DetectSource> = Vec::new();
    for candidate in candidates {
        if !sources.contains(&candidate.source()) {
            sources.push(candidate.source());
        }
    }
    let names: Vec<String> = sources.iter().map(ToString::to_string).collect();
    format!("{FOUND_IN} {}", names.join(SOURCE_SEPARATOR))
}

/// The report of `added`: one line per process.
pub fn added_rows(added: &[ProcessName]) -> Report {
    if added.is_empty() {
        return Report::new(INIT_TITLE, vec![Row::unlabeled(RowKind::Ok, NOTHING_ADDED)]);
    }
    Report::new(
        INIT_TITLE,
        added
            .iter()
            .map(|name| Row::unlabeled(RowKind::Ok, format!("{ADDED_MARK} {name}")))
            .collect(),
    )
}

/// How an interactive `init` ends once `added` were added, `autostart` or
/// not.
pub fn added_outro(added: &[ProcessName], autostart: bool) -> String {
    let count = added.len();
    let noun = if count == 1 { PROCESS } else { PROCESSES };
    let hint = if autostart { UP_HINT } else { START_HINT };
    format!("{ADDED} {count} {noun}; {hint}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{adapter::cli::report::plain, domain::value::CommandLine};

    /// A candidate `name` running `command` from `source`.
    fn candidate(name: &str, command: &str, source: DetectSource, suggested: bool) -> Candidate {
        Candidate::builder()
            .name(ProcessName::try_new(name).unwrap())
            .command(CommandLine::try_new(command).unwrap())
            .source(source)
            .suggested(suggested)
            .build()
    }

    /// The plain list checks the suggested candidates, names each command
    /// and source, and says how to add them; it writes nothing.
    #[test]
    fn the_plain_list_checks_the_suggested_ones() {
        let found = [
            candidate("dev", "npm run dev", DetectSource::PackageJson, true),
            candidate("lint", "npm run lint", DetectSource::PackageJson, false),
        ];

        assert_eq!(
            plain(&candidate_rows(&found)),
            format!(
                "[x] dev  npm run dev  package-json\n[ ] lint  npm run lint  package-json\n{YES_HINT}"
            )
        );
        assert_eq!(suggested(&found), vec![found[0].clone()]);
        assert_eq!(found_in(&found), "found processes in package-json");
    }

    /// With nothing found, it says how to add a process by hand.
    #[test]
    fn nothing_found_says_how_to_add_one() {
        assert!(plain(&candidate_rows(&[])).contains("muster add"));
    }

    /// The JSON lists each candidate with its source and whether it is
    /// suggested, and what was added.
    #[test]
    fn the_json_lists_candidates_and_additions() {
        let view = InitView::builder()
            .candidates(vec![candidate("web", "puma", DetectSource::Procfile, true)])
            .added(vec![ProcessName::try_new("web").unwrap()])
            .build();

        assert_eq!(
            serde_json::to_value(&view).unwrap(),
            serde_json::json!({
                "candidates": [{"name": "web", "command": "puma", "source": "procfile", "suggested": true}],
                "added": ["web"]
            })
        );
    }
}
