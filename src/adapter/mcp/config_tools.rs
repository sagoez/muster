//! The settings tools: the machine's muster settings, read and changed.

use rmcp::{handler::server::wrapper::Parameters, tool, tool_router};

use super::{
    config_get_arg::ConfigGetArg,
    config_set_arg::ConfigSetArg,
    process_fields::{KEY_FIELD, choice},
    server::{MusterMcp, json, structured},
    tool_error::ToolError,
    tool_result::ToolResult,
};
use crate::{
    adapter::client::{SettingView, SettingsView},
    domain::config::{SettingChange, SettingKey},
};

/// The setting tool, as a read-only refusal names it.
pub(super) const CONFIG_SET: &str = "config_set";

/// The setting key an agent typed as `raw`.
///
/// # Errors
/// Returns the caller's mistake for a key that names no setting, listing
/// every key.
fn setting_key(raw: &str) -> Result<SettingKey, ToolError> {
    choice(KEY_FIELD, raw)
}

#[tool_router(router = config_router, vis = "pub(super)")]
impl MusterMcp {
    /// Reads one setting, or all of them.
    #[tool(
        description = "Read the machine's muster settings. Takes key (optional). With key, \
            returns {key, value}; without, {settings: [{key, value}]} listing every setting. \
            Values are written as config_set reads them. An unknown key is a result flagged \
            isError that lists the valid keys."
    )]
    pub(super) async fn config_get(&self, Parameters(arg): Parameters<ConfigGetArg>) -> ToolResult {
        let key = arg.key().as_deref().map(setting_key).transpose()?;
        let settings = self.deps.settings().load()?;
        match key {
            Some(key) => structured(json(&SettingView::of(&settings, key))?, false),
            None => structured(json(&SettingsView::from(&settings))?, false),
        }
    }

    /// Changes one setting.
    #[tool(
        description = "Change one of the default-* settings, the defaults a new process takes. \
            Takes key and value, the value written as config_get shows it (true, 24x80, \
            on-failure, 5s). They apply to processes added, or for default-size started, \
            afterwards. Every other setting is the user's to change and is refused. The change \
            is shown to the user as a notice. Returns {key, value}. An unknown or refused key \
            or an invalid value is a result flagged isError that names the accepted form, as is \
            any change while the user's mcp-stack-edits setting is read-only."
    )]
    pub(super) async fn config_set(&self, Parameters(arg): Parameters<ConfigSetArg>) -> ToolResult {
        let key = setting_key(arg.key())?;
        if !key.agent_may_set() {
            return Err(ToolError::SettingNotForAgents(key).into());
        }
        let change = SettingChange::parse(key, arg.value())?;
        let author = self.identity.author().clone();
        let settings = self
            .blocking(move |deps| {
                deps.allow_stack_edit(CONFIG_SET)?;
                Ok(deps.editor().configure(&change, &author)?)
            })
            .await?;
        structured(json(&SettingView::of(&settings, key))?, false)
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;
    use crate::{
        adapter::mcp::test_support::{add, content, edits, fixture, refusal, register, runtime},
        domain::{config::McpStackEdits, port::SettingsReader},
    };

    /// `config_get` reads every setting or one; an unknown key lists the
    /// valid ones.
    #[test]
    fn config_get_reads_one_or_every_setting() {
        let fixture = fixture();
        runtime().block_on(async {
            let all = content(
                fixture
                    .server()
                    .config_get(Parameters(ConfigGetArg::builder().build()))
                    .await,
            );
            let listed = all["settings"].as_array().expect("a list of settings");
            assert_eq!(listed.len(), SettingKey::iter().count());

            let one = content(
                fixture
                    .server()
                    .config_get(Parameters(
                        ConfigGetArg::builder()
                            .key(Some("default-restart".to_string()))
                            .build(),
                    ))
                    .await,
            );
            assert_eq!(
                one,
                serde_json::json!({ "key": "default-restart", "value": "never" })
            );

            let unknown = refusal(
                fixture
                    .server()
                    .config_get(Parameters(
                        ConfigGetArg::builder()
                            .key(Some("restart".to_string()))
                            .build(),
                    ))
                    .await,
            );
            assert!(unknown.contains("default-restart"), "{unknown}");
        });
    }

    /// `config_set` stores a value a new process takes and tells a running
    /// service; an invalid value names the accepted form.
    #[test]
    fn config_set_stores_and_announces_a_setting() {
        let fixture = fixture();
        let server = &fixture.server();
        runtime().block_on(async {
            let stored = content(
                server
                    .config_set(Parameters(
                        ConfigSetArg::builder()
                            .key("default-restart".to_string())
                            .value("on-failure".to_string())
                            .build(),
                    ))
                    .await,
            );
            assert_eq!(
                stored,
                serde_json::json!({ "key": "default-restart", "value": "on-failure" })
            );

            let invalid = refusal(
                server
                    .config_set(Parameters(
                        ConfigSetArg::builder()
                            .key("default-stop-grace".to_string())
                            .value("0s".to_string())
                            .build(),
                    ))
                    .await,
            );
            assert!(invalid.contains("default-stop-grace"), "{invalid}");

            let added = content(
                server
                    .process_add(Parameters(add("web", "npm run dev")))
                    .await,
            );
            assert_eq!(added["process"]["restart"], "on-failure");

            content(
                server
                    .config_set(Parameters(
                        ConfigSetArg::builder()
                            .key("default-autostart".to_string())
                            .value("true".to_string())
                            .build(),
                    ))
                    .await,
            );
        });
        assert_eq!(*fixture.notifier().settings().lock().unwrap(), 2);
        assert_eq!(
            edits(&fixture).last().map(String::as_str),
            Some("agent test set default-autostart to true")
        );
    }

    /// A `config_set` to the value already stored answers it, but writes,
    /// logs and announces nothing.
    #[test]
    fn a_config_set_to_the_stored_value_changes_nothing() {
        let fixture = fixture();
        register(&fixture);
        let before = edits(&fixture);
        runtime().block_on(async {
            let answered = content(
                fixture
                    .server()
                    .config_set(Parameters(
                        ConfigSetArg::builder()
                            .key("default-restart".to_string())
                            .value("never".to_string())
                            .build(),
                    ))
                    .await,
            );
            assert_eq!(
                answered,
                serde_json::json!({ "key": "default-restart", "value": "never" })
            );
        });
        assert_eq!(*fixture.notifier().settings().lock().unwrap(), 0);
        assert_eq!(edits(&fixture), before);
    }

    /// `config_get` lists the idle stop with its seeded value, and
    /// `config_set` refuses it as the user's to change, storing nothing.
    #[test]
    fn config_get_lists_idle_stop_and_config_set_refuses_it() {
        let fixture = fixture();
        runtime().block_on(async {
            let all = content(
                fixture
                    .server()
                    .config_get(Parameters(ConfigGetArg::builder().build()))
                    .await,
            );
            assert!(
                all["settings"]
                    .as_array()
                    .unwrap()
                    .contains(&serde_json::json!({ "key": "idle-stop", "value": "30m" })),
                "{all}"
            );

            let refused = refusal(
                fixture
                    .server()
                    .config_set(Parameters(
                        ConfigSetArg::builder()
                            .key("idle-stop".to_string())
                            .value("never".to_string())
                            .build(),
                    ))
                    .await,
            );
            assert!(refused.contains("idle-stop"), "{refused}");
            assert!(refused.contains("cannot be changed over MCP"), "{refused}");
        });
        assert_eq!(
            fixture.store().load().unwrap().idle_stop().to_string(),
            "30m"
        );
        assert_eq!(*fixture.notifier().settings().lock().unwrap(), 0);
    }

    /// `config_set` refuses every key but the `default-*` ones, so an agent
    /// cannot lift its own guardrail, and nothing is stored.
    #[test]
    fn config_set_refuses_every_key_but_the_defaults() {
        let fixture = fixture();
        runtime().block_on(async {
            for key in SettingKey::iter().filter(|key| !key.agent_may_set()) {
                let refused = refusal(
                    fixture
                        .server()
                        .config_set(Parameters(
                            ConfigSetArg::builder()
                                .key(key.to_string())
                                .value("read-only".to_string())
                                .build(),
                        ))
                        .await,
                );
                assert!(refused.contains(&key.to_string()), "{refused}");
                assert!(refused.contains("cannot be changed over MCP"), "{refused}");
            }
        });
        assert_eq!(*fixture.notifier().settings().lock().unwrap(), 0);
        assert_eq!(
            *fixture.store().load().unwrap().mcp_stack_edits(),
            McpStackEdits::Allow
        );
    }
}
