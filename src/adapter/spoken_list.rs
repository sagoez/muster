//! A list of words as a sentence says it: `a`, `a and b`, `a, b or c`.

use strum::Display;

/// Between the items of a list but the last two.
const SEPARATOR: &str = ", ";

/// The word between the last two items of a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Display)]
#[strum(serialize_all = "kebab-case")]
pub enum Conjunction {
    /// Every item, together.
    And,
    /// Any one item.
    Or,
}

/// `items` as a sentence lists them, `conjunction` between the last two.
pub fn spoken_list(items: &[String], conjunction: Conjunction) -> String {
    match items.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} {conjunction} {last}", rest.join(SEPARATOR)),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `words` as owned strings.
    fn owned(words: &[&str]) -> Vec<String> {
        words.iter().map(ToString::to_string).collect()
    }

    /// One item stands alone, two are joined by the conjunction, and more
    /// are separated by commas but the last two.
    #[test]
    fn a_list_reads_as_a_sentence() {
        assert_eq!(spoken_list(&[], Conjunction::And), "");
        assert_eq!(spoken_list(&owned(&["a"]), Conjunction::Or), "a");
        assert_eq!(
            spoken_list(&owned(&["a", "b"]), Conjunction::And),
            "a and b"
        );
        assert_eq!(
            spoken_list(&owned(&["a", "b", "c"]), Conjunction::Or),
            "a, b or c"
        );
    }
}
