use getset::CopyGetters;
use typed_builder::TypedBuilder;

/// Text attributes of one cell.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, CopyGetters, TypedBuilder)]
pub struct CellFlags {
    /// Bold weight.
    #[getset(get_copy = "pub")]
    #[builder(default)]
    bold: bool,
    /// Dim intensity.
    #[getset(get_copy = "pub")]
    #[builder(default)]
    dim: bool,
    /// Italic slant.
    #[getset(get_copy = "pub")]
    #[builder(default)]
    italic: bool,
    /// Any underline style.
    #[getset(get_copy = "pub")]
    #[builder(default)]
    underline: bool,
    /// Inverse video.
    #[getset(get_copy = "pub")]
    #[builder(default)]
    inverse: bool,
    /// Concealed text.
    #[getset(get_copy = "pub")]
    #[builder(default)]
    hidden: bool,
    /// Struck-through text.
    #[getset(get_copy = "pub")]
    #[builder(default)]
    strikeout: bool,
}
