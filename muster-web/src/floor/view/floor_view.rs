use getset::Getters;
use muster_domain::{process::ProcessIcon, value::ProjectName};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::face_view::FaceView;

/// One project's floor as the page draws it.
#[derive(Clone, Debug, PartialEq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub(crate) struct FloorView {
    /// The project's name.
    name: ProjectName,
    /// Its folder, written from `~` when it is under the home folder.
    path: String,
    /// The mark the project wears.
    #[serde(with = "muster_protocol::wire::form")]
    icon: ProcessIcon,
    /// Its processes, in stack order.
    processes: Vec<FaceView>,
    /// Whether its stack did not read, so the page says so in place of its
    /// processes.
    unreadable: bool,
}
