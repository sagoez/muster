use nutype::nutype;
use uuid::Uuid;

/// Stable identity of a todo within a project, generated when the todo is added.
#[nutype(
    sanitize(trim),
    validate(not_empty),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        AsRef,
        Display,
        Serialize,
        Deserialize
    )
)]
pub struct TodoId(String);

impl TodoId {
    /// Creates a globally unique identity for a new todo.
    ///
    /// # Errors
    /// Returns the generated newtype's validation error if its invariant ever
    /// diverges from UUID's non-empty textual representation.
    pub fn generate() -> Result<Self, TodoIdError> {
        Self::try_new(Uuid::new_v4().to_string())
    }
}
