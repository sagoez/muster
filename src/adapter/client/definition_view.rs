use std::{collections::BTreeMap, path::PathBuf};

use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::{ready_view::ReadyView, stop_view::StopView};
use crate::domain::{
    config::ProcessSpec,
    process::RestartPolicy,
    value::{CommandLine, Description, ProcessName},
};

/// One process's stored definition, as `--json` and MCP show it.
#[derive(Debug, Clone, PartialEq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct DefinitionView {
    /// Its name within its project.
    #[getset(get = "pub")]
    name: ProcessName,
    /// The shell command it runs.
    #[getset(get = "pub")]
    command: CommandLine,
    /// Its working directory: relative to the project root, or absolute;
    /// none for the root itself.
    #[getset(get = "pub")]
    working_dir: Option<PathBuf>,
    /// Its one-line description.
    #[getset(get = "pub")]
    description: Option<Description>,
    /// The variables it is given, by name.
    #[getset(get = "pub")]
    environment: BTreeMap<String, String>,
    /// Whether `muster up` starts it.
    #[getset(get_copy = "pub")]
    autostart: bool,
    /// What happens when its child exits.
    #[getset(get_copy = "pub")]
    restart: RestartPolicy,
    /// How it is stopped.
    #[getset(get = "pub")]
    stop: StopView,
    /// What each run waits for before it counts as ready.
    #[getset(get = "pub")]
    ready: Option<ReadyView>,
}

impl From<&ProcessSpec> for DefinitionView {
    fn from(spec: &ProcessSpec) -> Self {
        Self {
            name: spec.name().clone(),
            command: spec.command().clone(),
            working_dir: spec.working_dir().clone(),
            description: spec.description().clone(),
            environment: spec
                .environment()
                .vars()
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
            autostart: *spec.autostart(),
            restart: *spec.restart(),
            stop: StopView::from(spec.stop()),
            ready: spec.ready().as_ref().map(ReadyView::from),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::domain::{
        config::Environment,
        process::{ReadyCheck, ReadyProbe, StopPolicy, StopSignal},
        value::{EnvName, EnvValue, PositiveDuration},
    };

    /// The definition reads as the documented JSON: snake_case keys,
    /// kebab-case values and the grace in seconds.
    #[test]
    fn a_definition_reads_as_documented() {
        let spec = ProcessSpec::builder()
            .name(ProcessName::try_new("web").unwrap())
            .command(CommandLine::try_new("npm run dev").unwrap())
            .working_dir(Some(PathBuf::from("frontend")))
            .environment(Environment::empty().with_var(
                EnvName::try_new("PORT").unwrap(),
                EnvValue::try_new("5173").unwrap(),
            ))
            .autostart(true)
            .restart(RestartPolicy::OnFailure)
            .stop(
                StopPolicy::builder()
                    .signal(StopSignal::Terminate)
                    .grace_period(PositiveDuration::try_new(Duration::from_millis(5_500)).unwrap())
                    .build(),
            )
            .build();

        assert_eq!(
            serde_json::to_value(DefinitionView::from(&spec)).unwrap(),
            serde_json::json!({
                "name": "web", "command": "npm run dev", "working_dir": "frontend",
                "description": null, "environment": {"PORT": "5173"}, "autostart": true,
                "restart": "on-failure", "stop": {"signal": "terminate", "grace": 5.5},
                "ready": null
            })
        );
        let regex = spec.with_ready(Some(
            ReadyCheck::builder()
                .probe(ReadyProbe::from_stored("regex", r"ready in \d+ms").unwrap())
                .timeout(PositiveDuration::try_new(Duration::from_secs(90)).unwrap())
                .build(),
        ));
        assert_eq!(
            serde_json::to_value(DefinitionView::from(&regex)).unwrap()["ready"],
            serde_json::json!({"log": r"ready in \d+ms", "regex": true, "timeout": "1m 30s"})
        );
    }
}
