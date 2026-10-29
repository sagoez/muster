use nutype::nutype;
use unicode_normalization::UnicodeNormalization;

use super::{process_name::ProcessName, unresolved_process::UnresolvedProcess};

/// A process name as a person typed it: trimmed, non-empty and in Unicode
/// normal form C, so `café` typed with a combining accent still names the
/// process configured with a precomposed one.
#[nutype(
    sanitize(trim, with = |typed: String| typed.nfc().collect()),
    validate(not_empty),
    derive(Debug, Clone, PartialEq, Eq, AsRef, Display, FromStr)
)]
pub struct ProcessRef(String);

impl ProcessRef {
    /// The one name among `names` this reference means: the byte-identical
    /// name when there is one, otherwise the single name equal to it once both
    /// are in normal form C. Case and inner whitespace always count.
    ///
    /// # Errors
    /// Returns [`UnresolvedProcess::Unknown`] when no name matches and
    /// [`UnresolvedProcess::Ambiguous`] when several match only by normal form.
    pub fn resolve<'a>(
        &self,
        names: impl IntoIterator<Item = &'a ProcessName>,
    ) -> Result<&'a ProcessName, UnresolvedProcess> {
        let equivalent: Vec<&ProcessName> = names
            .into_iter()
            .filter(|name| name.as_ref().nfc().eq(self.as_ref().chars()))
            .collect();
        if let Some(exact) = equivalent
            .iter()
            .find(|name| name.as_ref() == self.as_ref())
        {
            return Ok(exact);
        }
        match equivalent.as_slice() {
            [] => Err(UnresolvedProcess::Unknown(self.clone())),
            [only] => Ok(only),
            several => Err(UnresolvedProcess::Ambiguous(
                self.clone(),
                several.iter().map(|name| (*name).clone()).collect(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `café` with a precomposed `é`.
    const CAFE_COMPOSED: &str = "caf\u{e9}";
    /// `café` with `e` and a combining acute accent.
    const CAFE_DECOMPOSED: &str = "cafe\u{301}";

    /// The configured process names `raw`.
    fn names(raw: &[&str]) -> Vec<ProcessName> {
        raw.iter()
            .map(|name| ProcessName::try_new(*name).unwrap())
            .collect()
    }

    /// The reference a person typing `typed` makes.
    fn reference(typed: &str) -> ProcessRef {
        ProcessRef::try_new(typed).unwrap()
    }

    /// A name with spaces or accents resolves to itself, however the accent
    /// was encoded when it was typed.
    #[test]
    fn names_with_spaces_and_unicode_resolve() {
        let names = names(&["dev server", CAFE_COMPOSED, "api"]);

        assert_eq!(
            reference("dev server").resolve(&names).unwrap().as_ref(),
            "dev server"
        );
        assert_eq!(
            reference("  dev server ").resolve(&names).unwrap().as_ref(),
            "dev server",
            "surrounding whitespace is trimmed like a configured name"
        );
        assert_eq!(
            reference(CAFE_DECOMPOSED).resolve(&names).unwrap().as_ref(),
            CAFE_COMPOSED,
            "a decomposed accent names the composed process"
        );
        assert_eq!(
            reference(CAFE_COMPOSED).resolve(&names).unwrap().as_ref(),
            CAFE_COMPOSED
        );
    }

    /// Matching is exact apart from the Unicode encoding: case, inner
    /// whitespace and prefixes never match.
    #[test]
    fn near_names_do_not_resolve() {
        let names = names(&["dev server", "API"]);

        for typed in ["Dev Server", "dev  server", "dev", "api"] {
            assert_eq!(
                reference(typed).resolve(&names),
                Err(UnresolvedProcess::Unknown(reference(typed))),
                "{typed}"
            );
        }
    }

    /// Two configured names that differ only in encoding are told apart by
    /// an exact match, and are ambiguous otherwise.
    #[test]
    fn an_exact_match_wins_over_an_equivalent_one() {
        let both = names(&[CAFE_COMPOSED, CAFE_DECOMPOSED]);
        let reference = reference(CAFE_COMPOSED);

        assert_eq!(reference.resolve(&both).unwrap().as_ref(), CAFE_COMPOSED);
        let decomposed_only = names(&[CAFE_DECOMPOSED, CAFE_DECOMPOSED]);
        assert!(matches!(
            reference.resolve(&decomposed_only),
            Err(UnresolvedProcess::Ambiguous(_, candidates)) if candidates.len() == 2
        ));
    }

    #[test]
    fn a_blank_reference_is_refused() {
        assert!(ProcessRef::try_new("   ").is_err());
        assert!("   ".parse::<ProcessRef>().is_err());
    }
}
