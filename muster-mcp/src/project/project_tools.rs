//! The project tools: listing every registered project, and registering a
//! folder as a project with no process.

use muster_client::{ProjectIdentityView, ProjectTarget};
use rmcp::{handler::server::wrapper::Parameters, tool, tool_router};

use crate::{
    project::{listed_project::ListedProject, project_register_arg::ProjectRegisterArg},
    result::tool_result::ToolResult,
    runtime::{
        ToolName,
        server::{MusterMcp, json, object, structured, with_field, with_registered},
    },
};

/// Key of the project in the result.
const PROJECT: &str = "project";
/// Key of the listed projects in the result.
const PROJECTS: &str = "projects";

#[tool_router(router = project_router, vis = "pub(crate)")]
impl MusterMcp {
    /// Lists every registered project, marking the one calls default to.
    #[tool(
        description = "Every registered muster project, by name: {projects: [{name, root, \
            current}]}, current true for the one a call naming no project acts on. Projects are \
            often related repositories in other folders: every note, todo, value, lock and \
            process tool takes project, a project's name or folder, so what you learn about another \
            project is written to that project, where its agents read it."
    )]
    pub(crate) async fn project_list(&self) -> ToolResult {
        let projects = self
            .blocking(|deps| {
                let current = deps.project(None).ok();
                Ok(deps
                    .projects()
                    .projects()?
                    .iter()
                    .map(|project| ListedProject::of(project, current.as_ref()))
                    .collect::<Vec<_>>())
            })
            .await?;
        structured(with_field(object(), PROJECTS, json(&projects)?), false)
    }

    /// Registers a folder as a project with no process.
    #[tool(
        description = "Register a folder as a muster project with no process, so notes, todos, \
            values and locks work there. Takes project, a folder: this server's folder or one \
            inside it, read against the server's working directory; this server's folder when \
            absent. Refused for /, the home folder or a folder above it, a folder holding a \
            project, a folder inside a project (its project covers it already), and while the \
            user's mcp-stack-edits setting is read-only. The registration is shown to the user \
            as a notice. Returns {project: {name, root}}, with registered, the same, when this \
            call registered it; a folder that is a project already answers without registered, \
            writing nothing."
    )]
    pub(crate) async fn project_register(
        &self,
        Parameters(arg): Parameters<ProjectRegisterArg>,
    ) -> ToolResult {
        let author = self.identity.author().clone();
        let (project, newly) = self
            .blocking(move |deps| {
                let target = deps
                    .scope()
                    .folder_to_register(arg.project().as_deref(), deps.projects().as_ref())?;
                Ok(match target {
                    ProjectTarget::Registered(project) => (project, false),
                    ProjectTarget::Unregistered(root) => {
                        deps.allow_stack_edit(ToolName::ProjectRegister)?;
                        let registration = deps.editor().register(&root, &author)?;
                        let newly = *registration.newly_registered();
                        (registration.project().clone(), newly)
                    },
                })
            })
            .await?;
        structured(
            with_registered(
                with_field(
                    object(),
                    PROJECT,
                    json(&ProjectIdentityView::from(&project))?,
                ),
                newly.then_some(&project),
            )?,
            false,
        )
    }
}

#[cfg(test)]
mod tests {
    use muster_domain::{
        config::{McpStackEdits, SettingChange},
        coordination::Author,
        port::{ProjectStore, SettingsStore, StackReader, StackStore},
        value::ProjectRoot,
    };
    use serde_json::Value;
    use tempfile::TempDir;

    use super::*;
    use crate::runtime::test_support::{AGENT, content, edits, fixture, refusal, runtime};

    /// The key naming the project a call registered.
    const REGISTERED: &str = "registered";

    /// `project_register` of the folder `project` names, or the server's.
    fn register_arg(project: Option<&str>) -> Parameters<ProjectRegisterArg> {
        Parameters(
            ProjectRegisterArg::builder()
                .project(project.map(str::to_string))
                .build(),
        )
    }

    /// project_register registers the server's folder with no process,
    /// says so once and shows the user a notice of it; a second call
    /// answers the project without registered.
    #[test]
    fn project_register_registers_the_folder_once() {
        let fixture = fixture();
        let server = fixture.server();

        runtime().block_on(async {
            let first = content(server.project_register(register_arg(None)).await);
            let root = fixture.root().as_ref().display().to_string();
            assert_eq!(first[PROJECT]["name"], "shop");
            assert_eq!(first[REGISTERED]["root"], root);
            let again = content(server.project_register(register_arg(None)).await);
            assert_eq!(again[PROJECT]["root"], root);
            assert!(again.get(REGISTERED).is_none(), "{again}");
        });
        assert_eq!(fixture.store().projects().unwrap().len(), 1);
        assert_eq!(*fixture.notifier().registered().lock().unwrap(), vec![
            fixture.root().clone()
        ]);
        assert_eq!(edits(&fixture), vec![format!(
            "agent {AGENT} registered {} as the project shop",
            fixture.root().as_ref().display()
        )]);
    }

    /// project_list lists every registered project by name and folder,
    /// marking only the one the server's folder is in as current.
    #[test]
    fn project_list_lists_every_project_and_marks_the_current_one() {
        let fixture = fixture();
        let elsewhere = TempDir::new().unwrap();
        let api = ProjectRoot::try_new(elsewhere.path().canonicalize().unwrap()).unwrap();
        fixture.store().register(&api, &Author::human()).unwrap();

        let listed = runtime().block_on(async {
            content(fixture.server().project_register(register_arg(None)).await);
            content(fixture.server().project_list().await)
        });

        let projects = listed[PROJECTS].as_array().unwrap();
        assert_eq!(projects.len(), 2, "{listed}");
        let current: Vec<&Value> = projects
            .iter()
            .filter(|project| project["current"] == true)
            .collect();
        assert_eq!(current.len(), 1, "{listed}");
        assert_eq!(current[0]["name"], "shop");
        assert_eq!(
            current[0]["root"],
            fixture.root().as_ref().display().to_string()
        );
        assert!(
            projects
                .iter()
                .any(|project| project["root"] == api.as_ref().display().to_string()),
            "{listed}"
        );
    }

    /// A folder inside a project is refused, and so is any registration
    /// while mcp-stack-edits is read-only; neither registers anything.
    #[test]
    fn project_register_refuses_a_nested_folder_and_a_read_only_setting() {
        let fixture = fixture();
        fixture
            .store()
            .apply(
                &SettingChange::McpStackEdits(McpStackEdits::ReadOnly),
                &Author::human(),
            )
            .unwrap();

        runtime().block_on(async {
            let refused = refusal(fixture.server().project_register(register_arg(None)).await);
            assert!(
                refused.contains(ToolName::ProjectRegister.as_ref()),
                "{refused}"
            );
            assert!(refused.contains("mcp-stack-edits"), "{refused}");
        });
        assert!(fixture.store().projects().unwrap().is_empty());

        fixture
            .store()
            .apply(
                &SettingChange::McpStackEdits(McpStackEdits::Allow),
                &Author::human(),
            )
            .unwrap();
        runtime().block_on(async {
            content(fixture.server().project_register(register_arg(None)).await);
            let nested = refusal(
                fixture
                    .server()
                    .project_register(register_arg(Some("./frontend")))
                    .await,
            );
            assert!(nested.contains("inside the project 'shop'"), "{nested}");
            assert!(nested.contains("already cover it"), "{nested}");
            assert!(!nested.contains("process_add"), "{nested}");
            let outside = refusal(
                fixture
                    .server()
                    .project_register(register_arg(Some("..")))
                    .await,
            );
            assert!(
                outside.contains("only this server's folder or one inside it is registered"),
                "{outside}"
            );
            assert!(!outside.contains("process_add"), "{outside}");
        });
        assert_eq!(fixture.store().projects().unwrap().len(), 1);
    }

    /// A folder that is a project already answers as one with nothing
    /// written, even while mcp-stack-edits is read-only, since nothing is
    /// edited.
    #[test]
    fn project_register_of_a_project_answers_without_writing() {
        let fixture = fixture();
        runtime().block_on(async {
            content(fixture.server().project_register(register_arg(None)).await);
        });
        fixture
            .store()
            .apply(
                &SettingChange::McpStackEdits(McpStackEdits::ReadOnly),
                &Author::human(),
            )
            .unwrap();
        let revision = fixture.store().stack_revision(fixture.root()).unwrap();

        let again = runtime().block_on(async {
            content(fixture.server().project_register(register_arg(None)).await)
        });

        assert_eq!(again[PROJECT]["name"], "shop");
        assert!(again.get(REGISTERED).is_none(), "{again}");
        assert_eq!(
            fixture.store().stack_revision(fixture.root()).unwrap(),
            revision
        );
        assert_eq!(edits(&fixture).len(), 1, "only the first call is logged");
    }
}
