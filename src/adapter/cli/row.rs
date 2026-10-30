use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::{report::PLAIN_SEPARATOR, row_kind::RowKind};
use crate::adapter::sanitize::printable;

/// One line of command output: an optional labeled fact with a status kind.
#[derive(Clone, Debug, Getters, CopyGetters, TypedBuilder)]
pub struct Row {
    /// Status kind driving glyph and color.
    #[getset(get_copy = "pub")]
    kind: RowKind,
    /// Optional bold label column.
    #[builder(default)]
    #[getset(get = "pub")]
    label: Option<String>,
    /// The fact itself.
    #[getset(get = "pub")]
    detail: String,
}

impl Row {
    /// An unlabeled row of the given kind, its detail made [`printable`].
    pub fn unlabeled(kind: RowKind, detail: impl Into<String>) -> Self {
        Row::builder()
            .kind(kind)
            .detail(printable(&detail.into()))
            .build()
    }

    /// A labeled, glyphed row, its label and detail made [`printable`].
    pub fn labeled(kind: RowKind, label: impl Into<String>, detail: impl Into<String>) -> Self {
        Row::builder()
            .kind(kind)
            .label(Some(printable(&label.into())))
            .detail(printable(&detail.into()))
            .build()
    }

    /// The row in the plain line-per-fact layout: labeled rows keep the
    /// `glyph label: detail` shape, unlabeled rows are the bare detail, so
    /// piped output keeps one stable format.
    pub(super) fn plain_line(&self) -> String {
        match (&self.label, self.kind.glyph()) {
            (Some(label), Some(glyph)) => {
                format!("{glyph} {label}{PLAIN_SEPARATOR}{}", self.detail)
            },
            _ => self.detail.clone(),
        }
    }
}
