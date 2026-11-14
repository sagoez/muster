use getset::Getters;
use typed_builder::TypedBuilder;

/// What the composition root tells the CLI once `muster view` listens: the
/// address to open, and why the browser could not be opened, if it could
/// not.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ViewStarted {
    /// The address carrying the view's token.
    url: String,
    /// Why opening the default browser failed; none when it opened or was
    /// not asked to.
    browser_failure: Option<String>,
}
