/// Whether a run of the agent's CLI shows its output to the person.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentOutput {
    /// Its output passes through: the run the person asked for.
    Shown,
    /// Its output is dropped: a step muster takes around that run, such as
    /// putting an entry back.
    Hidden,
}
