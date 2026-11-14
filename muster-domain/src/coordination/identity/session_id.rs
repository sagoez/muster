use nutype::nutype;
use uuid::Uuid;

/// One MCP connection, fresh each time a server starts, so the locks it
/// takes are told apart from those of another connection under the same
/// agent label.
#[nutype(derive(Debug, Clone, PartialEq, Eq, Hash, AsRef, Display))]
pub struct SessionId(String);

impl SessionId {
    /// A new, unique session.
    pub fn generate() -> Self {
        Self::new(Uuid::new_v4().to_string())
    }
}
