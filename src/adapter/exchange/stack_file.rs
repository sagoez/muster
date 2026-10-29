use std::collections::HashSet;

use getset::Getters;
use serde::{Deserialize, Serialize};
use serde_yaml_ng::Value;
use typed_builder::TypedBuilder;

use super::{EntryError, EnvExport, ExchangeError, ProcessEntry};
use crate::domain::config::ProcessSpec;

/// The stack file `muster export` writes and `muster import` reads, in the
/// project root unless a path is given.
pub const STACK_FILE: &str = "muster.yml";
/// The only stack file version this muster reads and writes.
pub const STACK_FILE_VERSION: u64 = 1;
/// The comment every exported file starts with.
const STACK_FILE_HEADER: &str = "# muster stack: `muster import` loads it into a project.\n";
/// The top-level key holding the file's version.
const VERSION_KEY: &str = "version";
/// The key naming a process inside its entry.
const NAME_KEY: &str = "name";

/// A project's stack as an interchange file. It never carries the project's
/// root or name: the folder that imports it is the project. Reading parses
/// entries as raw YAML first so each error can name its entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
#[serde(deny_unknown_fields)]
#[getset(get = "pub")]
pub struct StackFile<P = ProcessEntry> {
    /// The format version, [`STACK_FILE_VERSION`].
    version: u64,
    /// The processes, in stack order.
    processes: Vec<P>,
}

/// Reads and validates a whole stack file into definitions, in file order.
///
/// # Errors
/// Returns [`ExchangeError::Malformed`] for text that is not a stack file,
/// [`ExchangeError::UnsupportedVersion`] for another version, and
/// [`ExchangeError::Entry`] naming the first invalid entry: an unknown key,
/// an empty command, an invalid or unfilled variable, a zero grace, or a
/// name an earlier entry already has.
pub fn read_stack_file(raw: &str) -> Result<Vec<ProcessSpec>, ExchangeError> {
    let document: Value = serde_yaml_ng::from_str(raw).map_err(malformed)?;
    check_version(&document)?;
    let file: StackFile<Value> = serde_yaml_ng::from_value(document).map_err(malformed)?;
    let mut keys = HashSet::new();
    file.processes
        .into_iter()
        .enumerate()
        .map(|(index, entry)| {
            let name = entry
                .get(NAME_KEY)
                .and_then(Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .map(str::to_owned);
            let refused = |source| ExchangeError::Entry {
                index,
                name: name.clone(),
                source,
            };
            let entry: ProcessEntry = serde_yaml_ng::from_value(entry).map_err(|error| {
                refused(EntryError::Shape {
                    reason: error.to_string(),
                })
            })?;
            let spec = ProcessSpec::try_from(entry).map_err(refused)?;
            if !keys.insert(spec.name().key()) {
                return Err(refused(EntryError::DuplicateName));
            }
            Ok(spec)
        })
        .collect()
}

/// Renders `processes` as a stack file, in their order, with variables
/// sorted and their values written or replaced by placeholders per `env`.
///
/// # Errors
/// Returns [`ExchangeError::Render`] when the YAML serializer fails.
pub fn write_stack_file(
    processes: &[ProcessSpec],
    env: EnvExport,
) -> Result<String, ExchangeError> {
    let file = StackFile::builder()
        .version(STACK_FILE_VERSION)
        .processes(
            processes
                .iter()
                .map(ProcessEntry::from)
                .map(|entry| match env {
                    EnvExport::Values => entry,
                    EnvExport::Placeholders => entry.without_env_values(),
                })
                .collect::<Vec<_>>(),
        )
        .build();
    let body = serde_yaml_ng::to_string(&file).map_err(|error| ExchangeError::Render {
        reason: error.to_string(),
    })?;
    Ok(format!("{STACK_FILE_HEADER}{body}"))
}

/// Refuses a document whose version is present but not
/// [`STACK_FILE_VERSION`], before its shape is checked, so a newer file
/// reports its version rather than the keys it added.
fn check_version(document: &Value) -> Result<(), ExchangeError> {
    match document.get(VERSION_KEY) {
        Some(version) => match version.as_u64() {
            Some(STACK_FILE_VERSION) => Ok(()),
            Some(found) => Err(ExchangeError::UnsupportedVersion { found }),
            None => Err(ExchangeError::Malformed {
                reason: format!("{VERSION_KEY} must be {STACK_FILE_VERSION}"),
            }),
        },
        None => Ok(()),
    }
}

/// The error for text that does not parse as a stack file.
fn malformed(error: serde_yaml_ng::Error) -> ExchangeError {
    ExchangeError::Malformed {
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use super::*;
    use crate::domain::{
        config::Environment,
        process::{RestartPolicy, StopPolicy, StopSignal},
        value::{CommandLine, Description, EnvName, EnvValue, PositiveDuration, ProcessName},
    };

    /// The two-process stack the design's example shows, values included.
    const EXAMPLE: &str = "\
# muster stack: `muster import` loads it into a project.
version: 1
processes:
- name: web
  command: npm run dev
  working_dir: frontend
  description: Vite dev server
  environment:
    PORT: '5173'
  autostart: true
  restart: on-failure
  stop:
    signal: terminate
    grace_period: 5s
- name: worker
  command: bin/worker
  autostart: false
  restart: always
  stop:
    signal: interrupt
    grace_period: 10s
";

    fn grace(seconds: u64) -> PositiveDuration {
        PositiveDuration::try_new(Duration::from_secs(seconds)).unwrap()
    }

    fn example() -> Vec<ProcessSpec> {
        vec![
            ProcessSpec::builder()
                .name(ProcessName::try_new("web").unwrap())
                .command(CommandLine::try_new("npm run dev").unwrap())
                .working_dir(Some(PathBuf::from("frontend")))
                .description(Some(Description::try_new("Vite dev server").unwrap()))
                .environment(Environment::empty().with_var(
                    EnvName::try_new("PORT").unwrap(),
                    EnvValue::try_new("5173").unwrap(),
                ))
                .autostart(true)
                .restart(RestartPolicy::OnFailure)
                .stop(
                    StopPolicy::builder()
                        .signal(StopSignal::Terminate)
                        .grace_period(grace(5))
                        .build(),
                )
                .build(),
            ProcessSpec::builder()
                .name(ProcessName::try_new("worker").unwrap())
                .command(CommandLine::try_new("bin/worker").unwrap())
                .environment(Environment::empty())
                .autostart(false)
                .restart(RestartPolicy::Always)
                .stop(
                    StopPolicy::builder()
                        .signal(StopSignal::Interrupt)
                        .grace_period(grace(10))
                        .build(),
                )
                .build(),
        ]
    }

    /// A one-process file whose entry body is `entry`, indented as an item.
    fn with_entry(entry: &str) -> String {
        format!(
            "version: 1\nprocesses:\n- {}\n",
            entry.trim().replace('\n', "\n  ")
        )
    }

    /// A valid entry with `extra` lines appended.
    fn entry_with(extra: &str) -> String {
        with_entry(&format!(
            "name: web\ncommand: npm run dev\nautostart: true\nrestart: never\nstop:\n  signal: terminate\n  grace_period: 5s\n{extra}"
        ))
    }

    fn refusal(raw: &str) -> String {
        read_stack_file(raw).unwrap_err().to_string()
    }

    #[test]
    fn writes_the_documented_format() {
        assert_eq!(
            write_stack_file(&example(), EnvExport::Values).unwrap(),
            EXAMPLE
        );
    }

    #[test]
    fn an_export_reads_back_as_the_same_stack() {
        assert_eq!(read_stack_file(EXAMPLE).unwrap(), example());
        let written = write_stack_file(&example(), EnvExport::Values).unwrap();
        assert_eq!(read_stack_file(&written).unwrap(), example());
        assert!(
            read_stack_file("version: 1\nprocesses: []\n")
                .unwrap()
                .is_empty()
        );
    }

    /// By default the file names each variable with a null placeholder, which
    /// import refuses until it is filled in; an empty value stays a value.
    #[test]
    fn placeholders_hide_values_and_import_refuses_them() {
        let written = write_stack_file(&example(), EnvExport::Placeholders).unwrap();

        assert!(written.contains("PORT: null"), "{written}");
        assert!(!written.contains("5173"));
        assert_eq!(
            refusal(&written),
            "processes[0] (web): PORT has no value; fill it in (or remove it) before importing"
        );
        let filled = written.replace("PORT: null", "PORT: ''");
        assert_eq!(
            read_stack_file(&filled).unwrap()[0].environment().vars()
                [&EnvName::try_new("PORT").unwrap()]
                .as_ref(),
            ""
        );
    }

    #[test]
    fn refuses_another_version() {
        assert_eq!(
            read_stack_file("version: 2\nprocesses: []\nports: {}\n").unwrap_err(),
            ExchangeError::UnsupportedVersion { found: 2 }
        );
        assert!(matches!(
            read_stack_file("version: one\nprocesses: []\n"),
            Err(ExchangeError::Malformed { .. })
        ));
        assert!(matches!(
            read_stack_file("processes: []\n"),
            Err(ExchangeError::Malformed { .. })
        ));
    }

    #[test]
    fn refuses_text_that_is_not_a_stack_file() {
        for raw in [
            "",
            "- a\n- b\n",
            "version: 1\nprocesses: []\nproject: shop\n",
            "{",
        ] {
            assert!(
                matches!(read_stack_file(raw), Err(ExchangeError::Malformed { .. })),
                "{raw:?}"
            );
        }
    }

    /// Every entry error names the entry by index and name.
    #[test]
    fn entry_errors_name_the_entry() {
        let unknown = refusal(&entry_with("ports: [3000]"));
        assert!(
            unknown.starts_with("processes[0] (web): unknown field `ports`"),
            "{unknown}"
        );

        let missing = refusal(&with_entry("name: web\ncommand: x\nrestart: never"));
        assert!(
            missing.starts_with("processes[0] (web): missing field"),
            "{missing}"
        );

        assert_eq!(
            refusal(&entry_with("environment:\n  MUSTER_PROJECT: x")),
            "processes[0] (web): 'MUSTER_PROJECT' is not a usable environment variable name: \
             it must be non-empty, contain no '=' or NUL, and not start with MUSTER_"
        );
        assert_eq!(
            refusal(&entry_with("environment:\n  KEY: \"a\\0b\"")),
            "processes[0] (web): the value of KEY contains a NUL byte"
        );
        assert_eq!(
            refusal(&entry_with("description: '  '")),
            "processes[0] (web): description must not be empty; leave the key out instead"
        );
        assert_eq!(
            refusal(&entry_with("").replace("grace_period: 5s", "grace_period: 0s")),
            "processes[0] (web): stop.grace_period must be longer than zero"
        );
        assert_eq!(
            refusal(&entry_with("").replace("command: npm run dev", "command: '  '")),
            "processes[0] (web): command must not be empty"
        );
    }

    #[test]
    fn an_entry_without_a_readable_name_is_named_by_index() {
        assert_eq!(
            refusal(&entry_with("").replace("name: web", "name: ''")),
            "processes[0]: name must not be empty"
        );
    }

    /// Names that read the same in normal form C are one name.
    #[test]
    fn refuses_a_repeated_name_by_key() {
        let raw = format!(
            "{}{}",
            entry_with("").replace("name: web", "name: \"caf\\u00e9\""),
            entry_with("")
                .replace("version: 1\nprocesses:\n", "")
                .replace("name: web", "name: \"cafe\\u0301\"")
        );

        assert_eq!(read_stack_file(&raw).unwrap_err(), ExchangeError::Entry {
            index: 1,
            name: Some("cafe\u{301}".to_string()),
            source: EntryError::DuplicateName,
        });
    }
}
