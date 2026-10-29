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
        coordination::{Author, KV_VALUE_MAX_CHARS, KvKey, NOTE_BODY_MAX_CHARS, ScratchpadKey},
        value::EpochSeconds,
    };

    /// A snapshot round-trips through YAML with its authors as labels.
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
        assert_eq!(
            serde_yaml_ng::from_str::<CoordinationSnapshot>(&yaml).unwrap(),
            snapshot
        );
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
                "version: 1\nscratchpads:\n- key: plan\n  body: {}\n  author: human\n  updated_at: 1\ntodos: []\nvalues:\n- key: blob\n  value: {}\n  author: human\n  updated_at: 1\n",
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
}
