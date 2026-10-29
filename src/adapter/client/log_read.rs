use std::path::PathBuf;

use getset::Getters;
use typed_builder::TypedBuilder;

use super::{log_text::LogText, project_identity_view::ProjectIdentityView};
use crate::domain::{project::Project, value::ProcessName};

/// What `muster logs` read of one process: its bytes exactly as written,
/// for a pipe to receive unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct LogRead {
    /// The process.
    name: ProcessName,
    /// The log file it was read from, or none for the live screen.
    log: Option<PathBuf>,
    /// What was read.
    bytes: Vec<u8>,
}

impl LogRead {
    /// The read as `--json` shows it, for `project`, with any invalid UTF-8
    /// replaced.
    pub fn text_in(&self, project: &Project) -> LogText {
        LogText::builder()
            .project(ProjectIdentityView::from(project))
            .name(self.name.clone())
            .log(self.log.clone())
            .text(String::from_utf8_lossy(&self.bytes).into_owned())
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::value::{ProjectName, ProjectRoot};

    /// The JSON names the project by name and folder, as every command does.
    #[test]
    fn the_json_names_the_project() {
        let project = Project::builder()
            .root(ProjectRoot::try_new(PathBuf::from("/p/shop")).unwrap())
            .name(ProjectName::try_new("shop").unwrap())
            .build();
        let read = LogRead::builder()
            .name(ProcessName::try_new("api").unwrap())
            .log(None)
            .bytes(b"ready\n".to_vec())
            .build();

        assert_eq!(
            serde_json::to_value(read.text_in(&project)).unwrap(),
            serde_json::json!({
                "project": {"name": "shop", "root": "/p/shop"},
                "name": "api",
                "log": null,
                "text": "ready\n"
            })
        );
    }
}
