use getset::Getters;
use typed_builder::TypedBuilder;

/// Opens the warning that the browser did not open, before why.
const NO_BROWSER_LEAD: &str = "could not open a browser: ";
/// Follows why, as the hint of that warning.
const NO_BROWSER_TAIL: &str = "; open the address above";

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

impl ViewStarted {
    /// The warning that the browser did not open and why, worded as every
    /// other warning, or `None` when it opened or was not asked to.
    pub fn browser_note(&self) -> Option<String> {
        self.browser_failure
            .as_ref()
            .map(|why| format!("{NO_BROWSER_LEAD}{why}{NO_BROWSER_TAIL}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A browser that failed to open is a lowercase warning with its hint
    /// after `; `, as every warning reads; one that opened says nothing.
    #[test]
    fn a_failed_browser_reads_as_a_warning() {
        let started = |failure: Option<&str>| {
            ViewStarted::builder()
                .url("http://127.0.0.1:1/v/t/".to_string())
                .browser_failure(failure.map(str::to_string))
                .build()
        };

        assert_eq!(
            started(Some("no xdg-open")).browser_note().as_deref(),
            Some("could not open a browser: no xdg-open; open the address above")
        );
        assert_eq!(started(None).browser_note(), None);
    }
}
