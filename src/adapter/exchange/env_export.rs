/// Whether an export writes environment values or only their names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvExport {
    /// Names only, each with a null placeholder `muster import` refuses until
    /// it is filled in, so a shared file carries no secrets.
    Placeholders,
    /// Names and their stored values (`muster export --with-env`).
    Values,
}
