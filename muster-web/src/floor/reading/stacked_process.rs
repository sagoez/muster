use getset::Getters;
use muster_domain::{process::ProcessIcon, value::ProcessName};
use typed_builder::TypedBuilder;

/// One process of a stored stack, with the icon resolved for it.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct StackedProcess {
    /// Its name within its project.
    name: ProcessName,
    /// The icon it wears.
    icon: ProcessIcon,
}
