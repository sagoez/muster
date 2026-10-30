use getset::Getters;
use typed_builder::TypedBuilder;

use super::EntryPosition;

/// One page of a project's notes or key-value entries in write order, and
/// where the next page starts when more follow.
#[derive(Debug, Clone, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct EntryPage<T> {
    /// The entries on this page.
    entries: Vec<T>,
    /// The last entry's position when more follow, to resume after.
    next: Option<EntryPosition>,
}

impl<T> EntryPage<T> {
    /// The first `limit` of `rows`, read one past the limit so whether more
    /// follow is known.
    #[must_use]
    pub fn of(mut rows: Vec<(EntryPosition, T)>, limit: usize) -> Self {
        let more = rows.len() > limit;
        rows.truncate(limit);
        let next = rows.last().map(|(position, _)| *position).filter(|_| more);
        Self {
            entries: rows.into_iter().map(|(_, entry)| entry).collect(),
            next,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The positions `raw` paired with themselves as entries.
    fn rows(raw: &[i64]) -> Vec<(EntryPosition, i64)> {
        raw.iter()
            .map(|at| (EntryPosition::try_new(*at).unwrap(), *at))
            .collect()
    }

    /// A row past the limit is dropped and the page resumes after its last
    /// entry; a page that holds every row has no next.
    #[test]
    fn a_page_resumes_only_when_more_follow() {
        let page = EntryPage::of(rows(&[3, 7, 9]), 2);
        assert_eq!(page.entries(), &[3, 7]);
        assert_eq!(page.next(), &Some(EntryPosition::try_new(7).unwrap()));

        let whole = EntryPage::of(rows(&[3, 7]), 2);
        assert_eq!(whole.entries(), &[3, 7]);
        assert_eq!(whole.next(), &None);
    }
}
