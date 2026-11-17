//! The line every registering command prints when it registers a folder,
//! or, in a dry run, would.

use muster_client::ProjectIdentityView;

use crate::cli::{
    rows::{row::Row, row_kind::RowKind},
    status::output::COLUMN_GAP,
};

/// Opens the line naming a newly registered project.
const REGISTERED: &str = "registered project";
/// Opens the line naming the project a dry run would register.
const WOULD_REGISTER: &str = "would register project";

/// The done line saying a command registered `project`.
pub fn registered_row(project: &ProjectIdentityView) -> Row {
    registration_row(RowKind::Ok, REGISTERED, project)
}

/// The line saying a dry run would register `project`, worded as
/// [`registered_row`] is but marked idle, since nothing was done.
pub fn would_register_row(project: &ProjectIdentityView) -> Row {
    registration_row(RowKind::Idle, WOULD_REGISTER, project)
}

/// A `kind` line opened by `opening`, then `project`'s name and folder.
fn registration_row(kind: RowKind, opening: &str, project: &ProjectIdentityView) -> Row {
    Row::unlabeled(
        kind,
        format!(
            "{opening} {}{COLUMN_GAP}{}",
            project.name(),
            project.root().display()
        ),
    )
}
