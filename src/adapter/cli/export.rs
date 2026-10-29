//! `muster export`: a project's processes written to a stack file.

use std::{fs, io};

use super::{
    error::CliError, export_view::ExportView, invocation::Invocation, output::project_title,
    report::Report, row::Row, row_kind::RowKind, stack_path::StackPath,
    text_file::write_atomically,
};
use crate::{
    adapter::exchange::{EnvExport, ExchangeError, write_stack_file},
    domain::config::ConfigError,
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
    let project = invocation.project(store.as_ref())?;
    let stack = store
        .stack(project.root())?
        .ok_or_else(|| ConfigError::UnknownProject {
            root: project.root().as_ref().clone(),
        })?;
    let env = if with_env {
        EnvExport::Values
    } else {
        EnvExport::Placeholders
    };
    let text =
        write_stack_file(project.root(), stack.processes(), env).map_err(|error| match error {
            ExchangeError::Unexportable { ref name, .. } => CliError::Unexportable {
                name: name.clone(),
                source: error,
            },
            other => CliError::Output(io::Error::other(other)),
        })?;
    let path = match StackPath::of(invocation, project.root().as_ref(), file)? {
        StackPath::Stdio => return Ok(Exported::Text(text)),
        StackPath::File(path) => path,
    };
    if !force && fs::symlink_metadata(&path).is_ok() {
        return Err(CliError::FileExists(path));
    }
    write_atomically(&path, &text).map_err(|source| CliError::WriteFile {
        path: path.clone(),
        source,
    })?;
    Ok(Exported::Written(
        ExportView::builder()
            .path(path)
            .processes(stack.processes().len())
            .project(project.name().clone())
            .build(),
    ))
}

/// The human report of `view`: how many processes went where.
pub fn export_rows(view: &ExportView) -> Report {
    let count = view.processes();
    let noun = if count == 1 { PROCESS } else { PROCESSES };
    let rows = vec![Row::unlabeled(
        RowKind::Ok,
        format!("{EXPORTED} {count} {noun} {TO} {}", view.path().display()),
    )];
    Report::new(project_title(EXPORT_WORD, view.project()), rows)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{adapter::cli::report::plain, domain::value::ProjectName};

    /// The count and file are named.
    #[test]
    fn an_export_says_what_went_where() {
        let view = ExportView::builder()
            .path(PathBuf::from("/home/me/shop/muster.yml"))
            .processes(1)
            .project(ProjectName::try_new("shop").unwrap())
            .build();

        assert_eq!(
            plain(&export_rows(&view)),
            "exported 1 process to /home/me/shop/muster.yml"
        );
        assert_eq!(
            serde_json::to_value(&view).unwrap(),
            serde_json::json!({"path": "/home/me/shop/muster.yml", "processes": 1})
        );
    }
}
