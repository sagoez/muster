use getset::{CopyGetters, Getters};
use muster_domain::value::printable;
use typed_builder::TypedBuilder;

use super::row_kind::RowKind;

/// One line of command output: an optional labeled fact with a status kind,
/// and the dimmed detail lines under it.
#[derive(Clone, Debug, Getters, CopyGetters, TypedBuilder)]
pub struct Row {
    /// Status kind driving the mark, its colour and the stream.
    #[getset(get_copy = "pub")]
    kind: RowKind,
    /// Optional label, padded to the widest so the details line up.
    #[builder(default)]
    #[getset(get = "pub")]
    label: Option<String>,
    /// Optional word after the label saying how its subject stands, padded
    /// to the widest so the facts after it line up.
    #[builder(default)]
    #[getset(get = "pub")]
    word: Option<String>,
    /// The fact itself.
    #[getset(get = "pub")]
    detail: String,
    /// Dimmed lines under the fact, such as why it failed.
    #[builder(default)]
    #[getset(get = "pub")]
    notes: Vec<String>,
}

impl Row {
    /// An unlabeled row of the given kind, its detail made [`printable`].
    pub fn unlabeled(kind: RowKind, detail: impl Into<String>) -> Self {
        Row::builder()
            .kind(kind)
            .detail(printable(&detail.into()))
            .build()
    }

    /// A labeled row, its label and detail made [`printable`].
    pub fn labeled(kind: RowKind, label: impl Into<String>, detail: impl Into<String>) -> Self {
        Row::builder()
            .kind(kind)
            .label(Some(printable(&label.into())))
            .detail(printable(&detail.into()))
            .build()
    }

    /// A labeled row whose `word` says how its subject stands, before its
    /// `facts`, each made [`printable`].
    pub fn stated(
        kind: RowKind,
        label: impl Into<String>,
        word: impl Into<String>,
        facts: impl Into<String>,
    ) -> Self {
        Row::builder()
            .kind(kind)
            .label(Some(printable(&label.into())))
            .word(Some(printable(&word.into())))
            .detail(printable(&facts.into()))
            .build()
    }

    /// The row with `note`, made [`printable`], as a dimmed line under it.
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(printable(&note.into()));
        self
    }
}
