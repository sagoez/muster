use getset::CopyGetters;
use typed_builder::TypedBuilder;

/// The words one agent's own CLI takes to install muster's plugin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct AgentCli {
    /// The agent's binary.
    program: &'static str,
    /// Its verb that installs a plugin.
    install: &'static str,
    /// Its verb that uninstalls one.
    uninstall: &'static str,
    /// What joins a marketplace's repository and the git ref it is pinned to.
    ref_mark: &'static str,
    /// What to tell the person once the plugin is in, when the agent asks
    /// something more of them.
    after_add: Option<&'static str>,
}
