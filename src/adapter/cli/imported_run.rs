use std::path::Path;

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::config::ProcessSpec;

/// Names the project root as a working directory.
const PROJECT_ROOT: &str = ".";
/// Between the columns of a line.
const GAP: &str = "  ";
/// Opens the working directory.
const IN: &str = "in";
/// Between what ran before and what will run.
const BECOMES: &str = "->";

/// What one process an import adds or changes will run, and for a change,
/// what it ran before: the human report shows both, so a file cannot slip
/// in a command unseen.
#[derive(Debug, Clone, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ImportedRun {
    /// The definition the file gives.
    spec: ProcessSpec,
    /// The definition it replaces, for a changed process.
    before: Option<ProcessSpec>,
}

impl ImportedRun {
    /// The command and working directory, each as `old -> new` where a
    /// change altered it: `npm run dev  in frontend`.
    pub fn describe(&self) -> String {
        let command = self.spec.command().to_string();
        let dir = shown_dir(self.spec.working_dir().as_deref());
        match &self.before {
            None => format!("{command}{GAP}{IN} {dir}"),
            Some(before) => format!(
                "{}{GAP}{IN} {}",
                becomes(before.command().as_ref(), &command),
                becomes(&shown_dir(before.working_dir().as_deref()), &dir)
            ),
        }
    }
}

/// `after`, or `before -> after` when they differ.
fn becomes(before: &str, after: &str) -> String {
    if before == after {
        after.to_string()
    } else {
        format!("{before} {BECOMES} {after}")
    }
}

/// A working directory as the report shows it: the root as `.`.
fn shown_dir(dir: Option<&Path>) -> String {
    dir.map_or_else(|| PROJECT_ROOT.to_string(), |dir| dir.display().to_string())
}
