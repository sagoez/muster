use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::value::ProcessName;

/// What an import adds, updates and removes, each in stack order.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ImportPlan {
    /// Imported names the project does not define yet, as the file spells them.
    added: Vec<ProcessName>,
    /// Defined names whose imported definition differs, as the file spells them.
    updated: Vec<ProcessName>,
    /// Defined names the import removes, as the stack spells them.
    removed: Vec<ProcessName>,
}

impl ImportPlan {
    /// Whether the import changes nothing.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.updated.is_empty() && self.removed.is_empty()
    }
}
