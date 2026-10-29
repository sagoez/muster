//! `muster export`: a project's processes written to a stack file.

use std::{fs, path::PathBuf};

use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::{
    error::CliError,
    invocation::Invocation,
    output::project_title,
    report::{Report, Row, RowKind},
    stack_path::StackPath,
    text_file::write_atomically,
};
use crate::{
    adapter::exchange::write_stack_file,
    domain::{config::ConfigError, port::StackStore, value::ProjectName},
};

/// The command word of `muster export`.
const EXPORT_WORD: &str = "export";
/// Opens the line saying what was written.
const EXPORTED: &str = "exported";
/// Between the count and the file.
const TO: &str = "to";
/// One process, counted.
const PROCESS: &str = "process";
/// Several processes, counted.
const PROCESSES: &str = "processes";
/// What an absolute working directory warns of.
const ABSOLUTE_DIR_HINT: &str =
    "works in an absolute folder, which will not resolve on another machine";

/// What `muster export` wrote, as `--json` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct ExportView {
    /// The file written.
    #[getset(get = "pub")]
    path: PathBuf,
    /// How many processes it holds.
    #[getset(get_copy = "pub")]
    processes: usize,
    /// The project exported.
    #[serde(skip)]
    #[getset(get = "pub")]
    project: ProjectName,
    /// The processes whose working directory is absolute.
    #[serde(skip)]
    #[getset(get = "pub")]
    absolute_dirs: Vec<String>,
}

/// What `muster export` produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exported {
    /// The file's text, to print alone on stdout.
    Text(String),
    /// What it wrote to a file.
    Written(ExportView),
}

/// Writes the invocation's project to `file` (the project's stack file when
/// absent, stdout for `-`), environment values only `with_env`.
///
/// # Errors
/// Returns [`CliError::FileExists`] for an existing file without `force`,
/// or the [`CliError`] of a project or file that cannot be used.
pub fn export(
    invocation: &Invocation<'_>,
    file: Option<&str>,
    force: bool,
    with_env: bool,
) -> Result<Exported, CliError> {
    let store = invocation.store()?;
    let project = invocation.project(&store)?;
    let stack = store
        .stack(project.root())?
        .ok_or_else(|| ConfigError::UnknownProject {
            root: project.root().as_ref().clone(),
        })?;
    let text = write_stack_file(stack.processes(), with_env);
    let path = match StackPath::of(invocation, project.root().as_ref(), file)? {
        StackPath::Stdio => return Ok(Exported::Text(text)),
        StackPath::File(path) => path,
    };
    if !force && fs::symlink_metadata(&path).is_ok() {
        return Err(CliError::FileExists(path));
    }
    write_atomically(&path, &text).map_err(|source| ConfigError::Write {
        path: path.clone(),
        source,
    })?;
    Ok(Exported::Written(
        ExportView::builder()
            .path(path)
            .processes(stack.processes().len())
            .project(project.name().clone())
            .absolute_dirs(
                stack
                    .processes()
                    .iter()
                    .filter(|spec| {
                        spec.working_dir()
                            .as_ref()
                            .is_some_and(|dir| dir.is_absolute())
                    })
                    .map(|spec| spec.name().to_string())
                    .collect(),
            )
            .build(),
    ))
}

/// The human report of `view`: how many processes went where, and which
/// will not resolve elsewhere.
pub fn export_rows(view: &ExportView) -> Report {
    let count = view.processes();
    let noun = if count == 1 { PROCESS } else { PROCESSES };
    let mut rows = vec![Row::unlabeled(
        RowKind::Ok,
        format!("{EXPORTED} {count} {noun} {TO} {}", view.path().display()),
    )];
    rows.extend(
        view.absolute_dirs()
            .iter()
            .map(|name| Row::labeled(RowKind::Hint, name.clone(), ABSOLUTE_DIR_HINT)),
    );
    Report::new(project_title(EXPORT_WORD, view.project()), rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::cli::report::plain;

    /// The count and file are named, and each absolute folder warned of.
    #[test]
    fn an_export_says_what_went_where() {
        let view = ExportView::builder()
            .path(PathBuf::from("/home/me/shop/muster.yml"))
            .processes(1)
            .project(ProjectName::try_new("shop").unwrap())
            .absolute_dirs(vec!["tools".to_string()])
            .build();

        assert_eq!(
            plain(&export_rows(&view)),
            format!("exported 1 process to /home/me/shop/muster.yml\n! tools: {ABSOLUTE_DIR_HINT}")
        );
        assert_eq!(
            serde_json::to_value(&view).unwrap(),
            serde_json::json!({"path": "/home/me/shop/muster.yml", "processes": 1})
        );
    }
}
