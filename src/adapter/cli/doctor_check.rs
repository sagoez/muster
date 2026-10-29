use serde::Serialize;

/// Row labels of the checks.
const DATABASE_LABEL: &str = "database";
const PROJECTS_LABEL: &str = "projects";
const PROCESSES_LABEL: &str = "processes";
const THIS_FOLDER_LABEL: &str = "this folder";
const OLD_FILES_LABEL: &str = "old files";
const COMPLETIONS_LABEL: &str = "completions";

/// What one `muster doctor` probe checks, named in kebab case under
/// `--json`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DoctorCheck {
    /// The database opens and its settings read.
    Database,
    /// The registered projects and whether their folders exist.
    Projects,
    /// Whether every stored process reads.
    Processes,
    /// The project the current folder belongs to.
    ThisFolder,
    /// Files an older muster left behind.
    OldFiles,
    /// Whether the shell registers muster's completions.
    Completions,
}

impl DoctorCheck {
    /// The label its row shows a person.
    pub fn label(self) -> &'static str {
        match self {
            Self::Database => DATABASE_LABEL,
            Self::Projects => PROJECTS_LABEL,
            Self::Processes => PROCESSES_LABEL,
            Self::ThisFolder => THIS_FOLDER_LABEL,
            Self::OldFiles => OLD_FILES_LABEL,
            Self::Completions => COMPLETIONS_LABEL,
        }
    }
}
