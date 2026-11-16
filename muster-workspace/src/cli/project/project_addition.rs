use getset::Getters;
use muster_client::ProjectIdentityView;
use serde::Serialize;
use typed_builder::TypedBuilder;

/// What `projects add` did, as `--json` shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ProjectAddition {
    /// The project the folder is.
    project: ProjectIdentityView,
    /// The project, when this call registered it: present only then, as on
    /// every command and tool that may register one.
    #[serde(skip_serializing_if = "Option::is_none")]
    registered: Option<ProjectIdentityView>,
}
