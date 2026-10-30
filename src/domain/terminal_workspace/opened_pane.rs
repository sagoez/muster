use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::value::WorkspacePaneId;

/// A pane a terminal workspace opened.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
pub struct OpenedPane {
    /// The new pane, made safe to print.
    #[getset(get = "pub")]
    pane: WorkspacePaneId,
}
