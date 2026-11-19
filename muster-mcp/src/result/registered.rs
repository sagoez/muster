use getset::Getters;
use muster_client::ProjectIdentityView;
use muster_domain::project::Project;
use serde::Serialize;
use typed_builder::TypedBuilder;

/// A result `view` naming the project the call registered, when it
/// registered one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Registered<View> {
    /// The result itself.
    #[serde(flatten)]
    view: View,
    /// The project this call registered, as `{name, root}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    registered: Option<ProjectIdentityView>,
}

impl<View> Registered<View> {
    /// `view`, naming `registered` when the call registered it.
    pub fn of(view: View, registered: Option<&Project>) -> Self {
        Self::builder()
            .view(view)
            .registered(registered.map(ProjectIdentityView::from))
            .build()
    }
}
