//! `muster init`: processes found in the files a folder already has, offered
//! to add. Detection only reads files; what is chosen is added in one
//! transaction, registering the folder.

use std::path::{Path, PathBuf};

use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::{
    error::CliError,
    report::{Report, Row, RowKind},
};
use crate::{
    adapter::{
        detect::{Candidate, DetectSource, detect},
        path::project_root,
    },
    application::StackEditor,
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

/// Opens the line naming the folder scanned.
const SCANNED: &str = "scanned";
/// Said when the current folder, not the one scanned, has processes of its
/// own to offer.
pub const HERE_HINT: &str =
    "this folder has processes of its own; `muster init --project .` offers them";

/// What `muster init` offered and added, as `--json` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct InitView {
    /// The folder whose files were scanned.
    #[getset(get = "pub")]
    folder: PathBuf,
    /// Every process offered.
    #[getset(get = "pub")]
    candidates: Vec<Candidate>,
    /// The ones added.
    #[getset(get = "pub")]
    added: Vec<ProcessName>,
    /// Whether the current folder, not the one scanned, offers processes of
    /// its own.
    #[serde(skip)]
    #[getset(get_copy = "pub")]
    offered_here: bool,
}

/// Whether `cwd`, a folder other than the scanned `root`, has files that
/// offer processes of their own.
pub fn offers_here(cwd: &Path, root: &ProjectRoot) -> bool {
    project_root(cwd).is_ok_and(|here| here != *root && !detect(&here).is_empty())
}

/// The line naming the folder `root` that was scanned.
pub fn scanned(root: &Path) -> String {
    format!("{SCANNED} {}", root.display())
}

/// The rows every `init` report opens with: the folder scanned, and when
/// the current folder offers processes of its own, how to scan it instead.
fn scan_rows(view: &InitView) -> Vec<Row> {
    let mut rows = vec![Row::unlabeled(RowKind::Dim, scanned(view.folder()))];
    if view.offered_here() {
        rows.push(Row::unlabeled(RowKind::Hint, HERE_HINT));
    }
    rows
}

/// The candidates found under `root`, leaving out names the project
/// already has.
///
/// # Errors
/// Returns the [`CliError`] of a stack that cannot be read.
pub fn offered(store: &dyn StackStore, root: &ProjectRoot) -> Result<Vec<Candidate>, CliError> {
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
    settings: &dyn SettingsStore,
    editor: &StackEditor<'_>,
    root: &ProjectRoot,
    chosen: &[Candidate],
    autostart: Option<bool>,
) -> Result<Vec<ProcessName>, CliError> {
    if chosen.is_empty() {
        return Ok(Vec::new());
    }
    let defaults = settings.load()?.process_defaults();
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
    let (_, plan) = editor.import(root, &specs, ImportMode::Strict)?;
    Ok(plan.added().clone())
}

/// The plain list of the candidates of `view`, the suggested ones checked,
/// and how to add them.
pub fn candidate_rows(view: &InitView) -> Report {
    if view.candidates().is_empty() {
        return nothing_found(view);
    }
    let mut rows = scan_rows(view);
    rows.extend(view.candidates().iter().map(|candidate| {
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
    }));
    rows.push(Row::unlabeled(RowKind::Dim, YES_HINT));
    Report::new(INIT_TITLE, rows)
}

/// The report of `view` when nothing was found to offer.
pub fn nothing_found(view: &InitView) -> Report {
    let mut rows = scan_rows(view);
    rows.push(Row::unlabeled(RowKind::Hint, NOTHING_FOUND));
    Report::new(INIT_TITLE, rows)
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

/// The report of what `view` added: one line per process.
pub fn added_rows(view: &InitView) -> Report {
    let mut rows = scan_rows(view);
    if view.added().is_empty() {
        rows.push(Row::unlabeled(RowKind::Ok, NOTHING_ADDED));
    }
    rows.extend(
        view.added()
            .iter()
            .map(|name| Row::unlabeled(RowKind::Ok, format!("{ADDED_MARK} {name}"))),
    );
    Report::new(INIT_TITLE, rows)
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
    use std::fs;

    use tempfile::TempDir;

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

    /// The view of scanning `/p/shop` and offering `candidates`, the current
    /// folder offering its own or not.
    fn scan(candidates: Vec<Candidate>, offered_here: bool) -> InitView {
        InitView::builder()
            .folder(PathBuf::from("/p/shop"))
            .candidates(candidates)
            .added(Vec::new())
            .offered_here(offered_here)
            .build()
    }

    /// The plain list names the folder scanned, checks the suggested
    /// candidates, names each command and source, and says how to add them;
    /// it writes nothing.
    #[test]
    fn the_plain_list_checks_the_suggested_ones() {
        let found = vec![
            candidate("dev", "npm run dev", DetectSource::PackageJson, true),
            candidate("lint", "npm run lint", DetectSource::PackageJson, false),
        ];

        assert_eq!(
            plain(&candidate_rows(&scan(found.clone(), false))),
            format!(
                "scanned /p/shop\n[x] dev  npm run dev  package-json\n[ ] lint  npm run lint  package-json\n{YES_HINT}"
            )
        );
        assert_eq!(suggested(&found), vec![found[0].clone()]);
        assert_eq!(found_in(&found), "found processes in package-json");
    }

    /// With nothing found, it names the folder scanned and says how to add
    /// a process by hand, and how to scan the current folder when it offers
    /// its own.
    #[test]
    fn nothing_found_says_how_to_add_one() {
        let rendered = plain(&candidate_rows(&scan(Vec::new(), false)));
        assert!(rendered.starts_with("scanned /p/shop\n"), "{rendered}");
        assert!(rendered.contains("muster add"), "{rendered}");
        assert!(!rendered.contains(HERE_HINT), "{rendered}");

        let rendered = plain(&nothing_found(&scan(Vec::new(), true)));
        assert!(rendered.contains("`muster init --project .`"), "{rendered}");
    }

    /// The current folder is pointed at only when it is not the folder
    /// scanned and its files offer processes.
    #[test]
    fn the_current_folder_is_offered_only_with_files_of_its_own() {
        let scratch = TempDir::new().unwrap();
        let shop = scratch.path().canonicalize().unwrap();
        let web = shop.join("web");
        fs::create_dir(&web).unwrap();
        let root = ProjectRoot::try_new(shop.clone()).unwrap();

        assert!(!offers_here(&web, &root));
        fs::write(web.join("Procfile"), "web: puma\n").unwrap();
        assert!(offers_here(&web, &root));
        fs::write(shop.join("Procfile"), "api: rails s\n").unwrap();
        assert!(!offers_here(&shop, &root));
    }

    /// The JSON names the folder scanned, lists each candidate with its
    /// source and whether it is suggested, and what was added.
    #[test]
    fn the_json_lists_candidates_and_additions() {
        let view = InitView::builder()
            .folder(PathBuf::from("/p/shop"))
            .candidates(vec![candidate("web", "puma", DetectSource::Procfile, true)])
            .added(vec![ProcessName::try_new("web").unwrap()])
            .offered_here(false)
            .build();

        assert_eq!(
            serde_json::to_value(&view).unwrap(),
            serde_json::json!({
                "folder": "/p/shop",
                "candidates": [{"name": "web", "command": "puma", "source": "procfile", "suggested": true}],
                "added": ["web"]
            })
        );
    }
}
