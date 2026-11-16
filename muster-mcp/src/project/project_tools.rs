//! The project tools: registering a folder as a project with no process.

use muster_client::{ProjectIdentityView, ProjectTarget};
use rmcp::{handler::server::wrapper::Parameters, tool, tool_router};

use crate::{
    project::project_register_arg::ProjectRegisterArg,
    result::tool_result::ToolResult,
    runtime::server::{MusterMcp, json, object, structured, with_field, with_registered},
};

/// The tool a read-only refusal names.
const PROJECT_REGISTER: &str = "project_register";
/// Key of the project in the result.
const PROJECT: &str = "project";

#[tool_router(router = project_router, vis = "pub(crate)")]
impl MusterMcp {
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
                        deps.allow_stack_edit(PROJECT_REGISTER)?;
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
        port::{ProjectStore, SettingsStore, StackReader},
    };

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
            assert!(refused.contains(PROJECT_REGISTER), "{refused}");
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
