use strum::{Display, EnumIter, EnumString};

/// How an imported stack meets the processes a project already has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumString, EnumIter)]
#[strum(serialize_all = "kebab-case")]
pub enum ImportMode {
    /// Refuse when any imported name is already defined.
    Strict,
    /// Imported entries replace same-named processes; the others stay.
    Merge,
    /// The stack becomes exactly the imported one.
    Replace,
}
