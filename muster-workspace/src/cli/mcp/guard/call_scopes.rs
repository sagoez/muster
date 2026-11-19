use getset::Getters;
use typed_builder::TypedBuilder;

use crate::cli::mcp::guard::{checkout_scope::CheckoutScope, todo_scope::TodoScope};

/// What a guarded call needs before it runs: a todo in progress in each
/// project, and each checkout's lock.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct CallScopes {
    /// The projects needing a todo in progress, each with its open todos.
    todo_scopes: Vec<TodoScope>,
    /// The checkouts needing their lock.
    checkouts: Vec<CheckoutScope>,
}
