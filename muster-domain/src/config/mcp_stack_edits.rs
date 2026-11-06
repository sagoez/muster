use strum::{Display, EnumIter, EnumString};

/// Whether an agent over MCP may add, change or remove processes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display, EnumString, EnumIter)]
#[strum(serialize_all = "kebab-case")]
pub enum McpStackEdits {
    /// Agents edit the stack, each edit raising a notice.
    Allow,
    /// Agents read the stack but every edit is refused.
    ReadOnly,
}

impl McpStackEdits {
    /// Whether an agent's stack edit goes ahead.
    pub fn allows(self) -> bool {
        match self {
            Self::Allow => true,
            Self::ReadOnly => false,
        }
    }
}
