use getset::CopyGetters;
use typed_builder::TypedBuilder;

/// Which processes `muster ps` lists and how.
#[derive(Debug, Clone, Copy, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct PsShape {
    /// Every loaded project, not just this one.
    all: bool,
    /// Each process's stored command too.
    commands: bool,
    /// Every column, as the table.
    wide: bool,
}
