use std::io;

use getset::Getters;
use muster_client::ProjectIdentityView;
use muster_domain::project::Project;
use serde::Serialize;
use serde_json::Value;
use typed_builder::TypedBuilder;

use crate::cli::{failure::error::CliError, rows::row::Row, stack::add::registered_row};

/// The JSON key naming the project a write registered.
const REGISTERED: &str = "registered";

/// What a coordination command answers: the rows a person reads, and the
/// value `--json` prints in their place.
#[derive(Clone, Debug, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Answer {
    /// The lines shown without `--json`.
    rows: Vec<Row>,
    /// The JSON shown with it.
    json: Value,
}

impl Answer {
    /// `rows` for a person, `view` for `--json`.
    ///
    /// # Errors
    /// Returns [`CliError::Output`] when `view` cannot be rendered as JSON.
    pub fn of(rows: Vec<Row>, view: &impl Serialize) -> Result<Self, CliError> {
        let json = serde_json::to_value(view)
            .map_err(|error| CliError::Output(io::Error::other(error)))?;
        Ok(Self { rows, json })
    }

    /// This answer to a write that registered `project`: the line saying
    /// so after its own, and `registered`, the project's name and folder,
    /// in its JSON object.
    ///
    /// # Errors
    /// Returns [`CliError::Output`] when the project cannot be rendered as
    /// JSON.
    pub fn registering(mut self, project: &Project) -> Result<Self, CliError> {
        let identity = ProjectIdentityView::from(project);
        let named = serde_json::to_value(&identity)
            .map_err(|error| CliError::Output(io::Error::other(error)))?;
        if let Value::Object(fields) = &mut self.json {
            fields.insert(REGISTERED.to_string(), named);
        }
        self.rows.push(registered_row(&identity));
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use muster_domain::value::{ProjectName, ProjectRoot};
    use serde_json::json;

    use super::*;
    use crate::cli::rows::{
        report::{Report, plain},
        row_kind::RowKind,
    };

    /// A write that registered its folder says so after its own line, and
    /// its JSON names the project.
    #[test]
    fn a_registering_write_names_the_project() {
        let project = Project::builder()
            .root(ProjectRoot::try_new(PathBuf::from("/home/me/notes")).unwrap())
            .name(ProjectName::try_new("notes").unwrap())
            .build();
        let answer = Answer::of(
            vec![Row::unlabeled(RowKind::Ok, "saved 'plan'")],
            &json!({
                "saved": "plan"
            }),
        )
        .unwrap()
        .registering(&project)
        .unwrap();

        insta::assert_snapshot!(
            "registering_write_rows",
            plain(&Report::new("muster note", answer.rows().clone()))
        );
        insta::assert_snapshot!(
            "registering_write_json",
            serde_json::to_string_pretty(answer.json()).unwrap()
        );
    }
}
