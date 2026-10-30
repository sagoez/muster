use getset::Getters;
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::adapter::client::{LogSliceView, ProjectIdentityView};

/// A filtered `muster logs --json`: the slice under the project it was read
/// from, named as every other command names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ProjectLogSlice {
    /// The project.
    project: ProjectIdentityView,
    /// The slice read.
    #[serde(flatten)]
    slice: LogSliceView,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{
        adapter::holder::LogSlice,
        domain::value::{OutputCursor, ProcessName, ProjectName},
    };

    /// The project is `{name, root}`, beside the slice's own fields.
    #[test]
    fn the_json_names_the_project_by_name_and_root() {
        let slice = LogSlice::builder()
            .from(OutputCursor::new(0))
            .cursor(OutputCursor::new(4))
            .lines(vec!["ok".to_string()])
            .omitted_lines(0)
            .rotated_away_bytes(0)
            .build();
        let report = ProjectLogSlice::builder()
            .project(
                ProjectIdentityView::builder()
                    .name(ProjectName::try_new("shop").unwrap())
                    .root(PathBuf::from("/p/shop"))
                    .build(),
            )
            .slice(LogSliceView::for_slice(
                ProcessName::try_new("api").unwrap(),
                &slice,
                None,
            ))
            .build();

        let json = serde_json::to_value(&report).unwrap();

        assert_eq!(
            json["project"],
            serde_json::json!({"name": "shop", "root": "/p/shop"})
        );
        assert_eq!(json["process"], "api");
        assert_eq!(json["text"], "ok");
        assert_eq!(json["lines_returned"], 1);
    }
}
