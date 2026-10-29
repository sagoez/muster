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
    /// replaced; a screen loses the blank rows below its last text.
    pub fn text_in(&self, project: &Project) -> LogText {
        let text = String::from_utf8_lossy(&self.bytes);
        let text = match self.log {
            Some(_) => text.as_ref(),
            None => text.trim_end(),
        };
        LogText::builder()
            .project(ProjectIdentityView::from(project))
            .process(self.name.clone())
            .log(self.log.clone())
            .text(text.to_string())
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
            .log(Some(PathBuf::from("/p/shop/api.log")))
            .bytes(b"ready\n".to_vec())
            .build();

        assert_eq!(
            serde_json::to_value(read.text_in(&project)).unwrap(),
            serde_json::json!({
                "project": {"name": "shop", "root": "/p/shop"},
                "process": "api",
                "log": "/p/shop/api.log",
                "text": "ready\n"
            })
        );
    }

    /// A screen read keeps its rows but not the blank ones below them.
    #[test]
    fn a_screen_loses_its_trailing_blank_rows() {
        let project = Project::builder()
            .root(ProjectRoot::try_new(PathBuf::from("/p/shop")).unwrap())
            .name(ProjectName::try_new("shop").unwrap())
            .build();
        let screen = LogRead::builder()
            .name(ProcessName::try_new("api").unwrap())
            .log(None)
            .bytes(b"$ ready\n  indented\n\n   \n\n".to_vec())
            .build();

        assert_eq!(screen.text_in(&project).text(), "$ ready\n  indented");
    }
}
