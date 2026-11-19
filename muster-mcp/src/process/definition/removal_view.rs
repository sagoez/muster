use getset::Getters;
use muster_client::{ProjectIdentityView, RemovedView};
use serde::Serialize;
use typed_builder::TypedBuilder;

/// What `process_remove` answers: the project, and the process removed and
/// whether it was stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct RemovalView {
    /// The project the process was removed from.
    project: ProjectIdentityView,
    /// The process removed, and how its stop went.
    #[serde(flatten)]
    removed: RemovedView,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use muster_domain::value::{ProcessName, ProjectName};
    use serde_json::json;

    use super::*;

    /// A removal names the project beside the process's own fields, not
    /// nested under a key of their own.
    #[test]
    fn the_removed_process_sits_beside_its_project() {
        let view = RemovalView::builder()
            .project(
                ProjectIdentityView::builder()
                    .name(ProjectName::try_new("shop").unwrap())
                    .root(PathBuf::from("/home/me/shop"))
                    .build(),
            )
            .removed(
                RemovedView::builder()
                    .name(ProcessName::try_new("web").unwrap())
                    .stopped(true)
                    .still_stopping(false)
                    .build(),
            )
            .build();

        assert_eq!(
            serde_json::to_value(view).unwrap(),
            json!({
                "project": { "name": "shop", "root": "/home/me/shop" },
                "name": "web",
                "stopped": true,
                "still_stopping": false,
            })
        );
    }
}
