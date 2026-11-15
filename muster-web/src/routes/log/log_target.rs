use getset::Getters;
use typed_builder::TypedBuilder;

/// The process a log route names, each query value percent-decoded once.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct LogTarget {
    /// The project's name.
    project: String,
    /// The process's name.
    name: String,
}
