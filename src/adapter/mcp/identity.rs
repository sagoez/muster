use std::{
    env,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::domain::coordination::{AgentId, Author, LockHolder, SessionId};

/// Who one MCP connection writes as, resolved once the client has named
/// itself, whether it has been told yet, and the session its locks belong to.
#[derive(Debug)]
pub struct Identity {
    /// The `--as` name.
    agent: Option<String>,
    /// The resolved author, fixed on first use.
    author: OnceLock<Author>,
    /// Whether a result has already echoed the author to the agent.
    introduced: AtomicBool,
    /// This connection, which owns the locks it takes.
    session: SessionId,
}

impl Identity {
    /// An identity that prefers `agent`, the `--as` name, in a new session.
    pub fn new(agent: Option<String>) -> Self {
        Self {
            agent,
            author: OnceLock::new(),
            introduced: AtomicBool::new(false),
            session: SessionId::generate(),
        }
    }

    /// This connection's session.
    pub fn session(&self) -> &SessionId {
        &self.session
    }

    /// Who takes and releases locks: the author, through this session.
    pub fn lock_holder(&self) -> LockHolder {
        LockHolder::builder()
            .author(self.author().clone())
            .session(self.session.clone())
            .build()
    }

    /// Fixes the author now that the client named itself `client`; a call
    /// after the author was fixed changes nothing.
    pub fn settle(&self, client: Option<&str>) -> &Author {
        self.author.get_or_init(|| {
            AgentId::resolve(self.agent.as_deref(), &|key| env::var(key).ok(), client).author()
        })
    }

    /// The author coordination writes carry.
    pub fn author(&self) -> &Author {
        self.settle(None)
    }

    /// The author's label the first time a write result asks, so the agent
    /// learns which entries are its own; none afterwards.
    pub fn introduce(&self) -> Option<String> {
        (!self.introduced.swap(true, Ordering::Relaxed)).then(|| self.author().label().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The client's name joins the `--as` name, the author is fixed once,
    /// and only the first write is told who it is.
    #[test]
    fn the_author_is_fixed_once_and_introduced_once() {
        let identity = Identity::new(Some("builder".to_string()));
        assert_eq!(
            identity.settle(Some("claude-code")).label(),
            "claude-code@builder"
        );
        assert_eq!(
            identity.settle(Some("other")).label(),
            "claude-code@builder"
        );
        assert_eq!(identity.introduce().as_deref(), Some("claude-code@builder"));
        assert_eq!(identity.introduce(), None);
    }
}
