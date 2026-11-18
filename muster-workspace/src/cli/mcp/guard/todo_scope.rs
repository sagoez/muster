use getset::Getters;
use muster_domain::{coordination::Todo, project::Project};
use typed_builder::TypedBuilder;

/// A project a call needs a todo in progress in, with its todos.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct TodoScope {
    /// The project.
    project: Project,
    /// Its todos, every one read, so no number of others hides the agent's.
    todos: Vec<Todo>,
}
