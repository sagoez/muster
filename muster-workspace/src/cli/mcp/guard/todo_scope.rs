use getset::Getters;
use muster_domain::{coordination::Todo, project::Project};
use typed_builder::TypedBuilder;

/// A project a call needs a todo in progress in, with its open todos.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct TodoScope {
    /// The project.
    project: Project,
    /// Its open todos.
    todos: Vec<Todo>,
}
