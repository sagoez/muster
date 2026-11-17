/// Whether `muster mcp add` puts muster's plugin in or `remove` takes it
/// out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginChange {
    /// `muster mcp add`.
    Add,
    /// `muster mcp remove`.
    Remove,
}
