use muster_domain::coordination::Lock;

/// What the guard answers a call before it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Let it run, saying nothing.
    Allow,
    /// Let it run, and renew this checkout lock, which the agent holds,
    /// through the connection that took it.
    Renew(Lock),
    /// Refuse it, with the reason, which says how to proceed.
    Deny(String),
}
