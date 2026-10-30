use getset::{CopyGetters, Getters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use crate::{
    adapter::wire::form_list,
    domain::coordination::{KeyValue, Scratchpad, Todo},
};

/// The snapshot schema version, checked on import so a file from a newer
/// muster is refused rather than half-read.
pub const SNAPSHOT_VERSION: u8 = 1;

/// A portable snapshot of one project's coordination state. SQLite is the
/// operational store; this YAML form is the interchange format - a backup, a
/// file to commit alongside a repo, or a way to move state between machines.
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, TypedBuilder,
)]
#[serde(deny_unknown_fields)]
pub struct CoordinationSnapshot {
    /// The schema version the file was written with.
    #[getset(get_copy = "pub")]
    #[builder(default = SNAPSHOT_VERSION)]
    version: u8,
    /// Every scratchpad.
    #[getset(get = "pub")]
    #[serde(with = "form_list")]
    scratchpads: Vec<Scratchpad>,
    /// Every todo.
    #[getset(get = "pub")]
    #[serde(with = "form_list")]
    todos: Vec<Todo>,
    /// Every key-value entry.
    #[getset(get = "pub")]
    #[serde(with = "form_list")]
    values: Vec<KeyValue>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        coordination::{
            Author, KV_KEY_MAX_CHARS, KV_VALUE_MAX_CHARS, KvKey, NOTE_BODY_MAX_CHARS,
            NOTE_KEY_MAX_CHARS, ScratchpadKey, TODO_DEPS_MAX,
        },
        value::EpochSeconds,
    };

    /// A snapshot round-trips through YAML with its authors as labels and
    /// its times as RFC 3339.
    #[test]
    fn a_snapshot_round_trips_through_yaml() {
        let snapshot = CoordinationSnapshot::builder()
            .scratchpads(vec![
                Scratchpad::builder()
                    .key(ScratchpadKey::try_new("plan").unwrap())
                    .body("ship it".to_string())
                    .author(Author::agent("claude"))
                    .updated_at(EpochSeconds::new(7))
                    .build(),
            ])
            .todos(Vec::new())
            .values(vec![
                KeyValue::builder()
                    .key(KvKey::try_new("port").unwrap())
                    .value("3000".to_string())
                    .author(Author::human())
                    .updated_at(EpochSeconds::new(9))
                    .build(),
            ])
            .build();

        let yaml = serde_yaml_ng::to_string(&snapshot).unwrap();

        assert!(yaml.contains("author: claude"), "{yaml}");
        assert!(yaml.contains("author: human"), "{yaml}");
        assert!(
            yaml.contains("updated_at: 1970-01-01T00:00:07Z"),
            "times are RFC 3339: {yaml}"
        );
        assert_eq!(
            serde_yaml_ng::from_str::<CoordinationSnapshot>(&yaml).unwrap(),
            snapshot
        );
    }

    /// An imported author label is trimmed and `human` reads in any case; a
    /// blank one is refused.
    #[test]
    fn an_imported_author_reads_trimmed_and_a_blank_one_is_refused() {
        let file = |author: &str| {
            format!(
                "version: 1\nscratchpads:\n- key: plan\n  body: b\n  author: '{author}'\n  updated_at: 1970-01-01T00:00:01Z\ntodos: []\nvalues: []\n"
            )
        };

        let read = serde_yaml_ng::from_str::<CoordinationSnapshot>(&file(" HUMAN ")).unwrap();
        assert_eq!(read.scratchpads()[0].author(), &Author::Human);
        let blank = serde_yaml_ng::from_str::<CoordinationSnapshot>(&file("  "))
            .unwrap_err()
            .to_string();
        assert!(blank.contains("author label must not be empty"), "{blank}");
    }

    /// An imported time written at an offset or with a fraction of a second
    /// reads as the UTC second it names.
    #[test]
    fn an_imported_time_reads_at_any_rfc3339_offset() {
        let file = |at: &str| {
            format!(
                "version: 1\nscratchpads:\n- key: plan\n  body: b\n  author: human\n  updated_at: {at}\ntodos: []\nvalues: []\n"
            )
        };
        let read = |at: &str| {
            *serde_yaml_ng::from_str::<CoordinationSnapshot>(&file(at))
                .unwrap()
                .scratchpads()[0]
                .updated_at()
        };

        let utc = read("1970-01-01T00:00:07Z");
        assert_eq!(read("1970-01-01T02:00:07+02:00"), utc);
        assert_eq!(read("1970-01-01T00:00:07.9Z"), utc);
    }

    /// A snapshot with a field this muster does not know is refused.
    #[test]
    fn an_unknown_field_is_refused() {
        let yaml = "version: 1\nscratchpads: []\ntodos: []\nvalues: []\nextra: 1\n";

        assert!(serde_yaml_ng::from_str::<CoordinationSnapshot>(yaml).is_err());
    }

    /// A note body or a value past the cap a write is held to is refused on
    /// import too, naming its key; one at the cap is read.
    #[test]
    fn an_oversized_note_or_value_is_refused() {
        let file = |body: usize, value: usize| {
            format!(
                "version: 1\nscratchpads:\n- key: plan\n  body: {}\n  author: human\n  updated_at: 1970-01-01T00:00:01Z\ntodos: []\nvalues:\n- key: blob\n  value: {}\n  author: human\n  updated_at: 1970-01-01T00:00:01Z\n",
                "x".repeat(body),
                "y".repeat(value)
            )
        };
        let read = |yaml: String| serde_yaml_ng::from_str::<CoordinationSnapshot>(&yaml);

        assert!(read(file(NOTE_BODY_MAX_CHARS, KV_VALUE_MAX_CHARS)).is_ok());
        let note = read(file(NOTE_BODY_MAX_CHARS + 1, 1))
            .unwrap_err()
            .to_string();
        assert!(note.contains("note 'plan'"), "{note}");
        let value = read(file(1, KV_VALUE_MAX_CHARS + 1))
            .unwrap_err()
            .to_string();
        assert!(value.contains("value 'blob'"), "{value}");
    }

    /// A key past its maximum, or a todo with too many dependencies, is
    /// refused on import naming the limit.
    #[test]
    fn an_oversized_key_or_dependency_list_is_refused() {
        let read = |yaml: String| {
            serde_yaml_ng::from_str::<CoordinationSnapshot>(&yaml)
                .map_err(|error| error.to_string())
        };
        let keyed = |note: usize, value: usize| {
            format!(
                "version: 1\nscratchpads:\n- key: {}\n  body: b\n  author: human\n  updated_at: 1970-01-01T00:00:01Z\ntodos: []\nvalues:\n- key: {}\n  value: v\n  author: human\n  updated_at: 1970-01-01T00:00:01Z\n",
                "k".repeat(note),
                "k".repeat(value)
            )
        };
        assert!(read(keyed(NOTE_KEY_MAX_CHARS, KV_KEY_MAX_CHARS)).is_ok());
        for refused in [
            read(keyed(NOTE_KEY_MAX_CHARS + 1, 1)),
            read(keyed(1, KV_KEY_MAX_CHARS + 1)),
        ] {
            let refused = refused.unwrap_err();
            assert!(refused.contains("too long"), "{refused}");
        }
        let todo = |deps: usize| {
            format!(
                "version: 1\nscratchpads: []\ntodos:\n- id: t\n  title: ship\n  status: pending\n  deps: [{}]\n  claim: null\n  author: human\n  updated_at: 1970-01-01T00:00:01Z\nvalues: []\n",
                vec!["a"; deps].join(", ")
            )
        };
        assert!(
            read(todo(TODO_DEPS_MAX)).is_ok(),
            "{:?}",
            read(todo(TODO_DEPS_MAX))
        );
        let many = read(todo(TODO_DEPS_MAX + 1)).unwrap_err();
        assert!(
            many.contains(&format!("at most {TODO_DEPS_MAX} are allowed")),
            "{many}"
        );
    }
}
