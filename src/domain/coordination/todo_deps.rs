use nutype::nutype;

use super::TodoId;

/// Most dependencies one todo is written with.
pub const TODO_DEPS_MAX: usize = 32;

/// Whether `deps` holds no more than [`TODO_DEPS_MAX`] ids.
fn is_bounded(deps: &[TodoId]) -> bool {
    deps.len() <= TODO_DEPS_MAX
}

/// The ids a todo is written to depend on, at most [`TODO_DEPS_MAX`] of them,
/// so one write cannot flood a listing.
#[nutype(
    validate(predicate = |deps| is_bounded(deps)),
    derive(Debug, Clone, PartialEq, Eq, AsRef)
)]
pub struct TodoDeps(Vec<TodoId>);

#[cfg(test)]
mod tests {
    use super::*;

    /// The ids `count` dependencies name.
    fn ids(count: usize) -> Vec<TodoId> {
        (0..count)
            .map(|at| TodoId::try_new(at.to_string()).unwrap())
            .collect()
    }

    /// Up to the maximum is kept; one past it is refused.
    #[test]
    fn dependencies_are_bounded() {
        assert!(TodoDeps::try_new(ids(TODO_DEPS_MAX)).is_ok());
        assert!(TodoDeps::try_new(ids(TODO_DEPS_MAX + 1)).is_err());
    }
}
