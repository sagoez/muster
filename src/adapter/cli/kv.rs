use serde_json::json;

use super::{
    answer::Answer, error::CliError, kv_command::KvCommand, report::attribution, row::Row,
    row_kind::RowKind,
};
use crate::{
    adapter::{
        client::{ListedValue, PageSize},
        wire::WireForm,
    },
    domain::{
        coordination::{Author, KV_VALUE_MAX_CHARS, KvKey, KvValue},
        port::CoordinationStore,
        value::ProjectRoot,
    },
};

/// What a refusal calls an entry's value.
const KV_VALUE: &str = "value";
/// Line printed when the project has no key-value entries yet.
const EMPTY_NOTE: &str = "no values; write one with `muster kv set <key> <value>`";
/// Columns between a listed entry's key and value.
const FIELD_GAP: &str = "  ";
/// The JSON key of the listed entries.
const ENTRIES: &str = "entries";
/// The JSON key of whether entries follow the listed page.
const MORE: &str = "more";
/// What follows a listed value that was cut short.
const CUT_MARK: &str = "...";
/// Line printed under a listing that more entries follow.
const MORE_NOTE: &str = "more follow; pass a larger --limit to list them";
/// The JSON key of the key a write saved.
const SAVED: &str = "saved";
/// The JSON key of the key a delete named.
const KEY: &str = "key";
/// The JSON key of whether a delete found something.
const DELETED: &str = "deleted";

/// Runs a `muster kv` action against `store` for `project`, returning what to
/// print. Absent a subcommand, lists the project's first `limit` entries.
///
/// # Errors
/// Returns [`CliError`] when a key is invalid, a requested value is absent, or the
/// store cannot be read or written.
pub fn kv(
    command: Option<KvCommand>,
    limit: PageSize,
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
    author: &Author,
) -> Result<Answer, CliError> {
    match command {
        None => list(store, project, limit),
        Some(KvCommand::Get { key }) => get(store, project, &key),
        Some(KvCommand::Set { key, value }) => set(store, project, author, &key, &value),
        Some(KvCommand::Rm { key }) => remove(store, project, &key),
    }
}

/// Parses a user-supplied key into a validated [`KvKey`].
///
/// # Errors
/// Returns [`CliError::InvalidKvKey`] when the key is blank or whitespace-only.
fn parse_key(key: &str) -> Result<KvKey, CliError> {
    KvKey::try_new(key).map_err(|_| CliError::InvalidKvKey(key.to_string()))
}

/// Lists the first `limit` entries for `project` as `key  value`, one per
/// line, each value cut as `kv_read` cuts it, saying when more follow.
///
/// # Errors
/// Returns [`CliError`] when the store cannot be read.
fn list(
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
    limit: PageSize,
) -> Result<Answer, CliError> {
    let entries = store.values(project)?;
    let page: Vec<ListedValue> = entries
        .iter()
        .take(limit.into_inner())
        .map(ListedValue::from)
        .collect();
    let more = entries.len() > page.len();
    let mut rows: Vec<Row> = page
        .iter()
        .map(|listed| {
            let entry = listed.entry();
            Row::unlabeled(
                RowKind::Plain,
                format!(
                    "{key}{FIELD_GAP}{value}{cut}{attribution}",
                    key = entry.key().as_ref(),
                    value = entry.value(),
                    cut = if listed.truncated() { CUT_MARK } else { "" },
                    attribution = attribution(entry.author()),
                ),
            )
        })
        .collect();
    if entries.is_empty() {
        rows.push(Row::unlabeled(RowKind::Hint, EMPTY_NOTE));
    }
    if more {
        rows.push(Row::unlabeled(RowKind::Hint, MORE_NOTE));
    }
    Answer::of(rows, &json!({ ENTRIES: page, MORE: more }))
}

/// Prints the value under `key`, one row per line so the framed layout stays
/// intact and piped output reproduces the value verbatim.
///
/// # Errors
/// Returns [`CliError::InvalidKvKey`] for a blank key, [`CliError::UnknownValue`]
/// when no value is stored under it, or a store read failure.
fn get(
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
    key: &str,
) -> Result<Answer, CliError> {
    let parsed = parse_key(key)?;
    let entry = store
        .value(project, &parsed)?
        .ok_or_else(|| CliError::UnknownValue(key.to_string()))?;
    let rows = entry
        .value()
        .lines()
        .map(|line| Row::unlabeled(RowKind::Plain, line.to_string()))
        .collect();
    Answer::of(rows, &entry.to_wire())
}

/// Creates or replaces the entry under `key` with `value`.
///
/// # Errors
/// Returns [`CliError::InvalidKvKey`] for a blank key, [`CliError::TooLong`]
/// for a value past [`KV_VALUE_MAX_CHARS`], or a store write failure.
fn set(
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
    author: &Author,
    key: &str,
    value: &str,
) -> Result<Answer, CliError> {
    let parsed = parse_key(key)?;
    let value = KvValue::try_new(value).map_err(|_| CliError::TooLong {
        what: KV_VALUE,
        max: KV_VALUE_MAX_CHARS,
        found: value.chars().count(),
    })?;
    store.set_value(project, author, &parsed, &value)?;
    Answer::of(
        vec![Row::unlabeled(
            RowKind::Ok,
            format!("saved '{}'", parsed.as_ref()),
        )],
        &json!({ SAVED: parsed.as_ref() }),
    )
}

/// Deletes the entry under `key`. Deleting an absent entry is not an error.
///
/// # Errors
/// Returns [`CliError::InvalidKvKey`] for a blank key, or a store write failure.
fn remove(
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
    key: &str,
) -> Result<Answer, CliError> {
    let parsed = parse_key(key)?;
    let deleted = store.delete_value(project, &parsed)?;
    let row = if deleted {
        Row::unlabeled(RowKind::Ok, format!("removed '{}'", parsed.as_ref()))
    } else {
        Row::unlabeled(
            RowKind::Hint,
            format!("no value '{}' to remove", parsed.as_ref()),
        )
    };
    Answer::of(
        vec![row],
        &json!({ KEY: parsed.as_ref(), DELETED: deleted }),
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{super::memory_coordination::MemoryCoordination, *};
    use crate::adapter::client::LISTED_VALUE_CHARS;

    /// The project every test entry belongs to.
    fn project() -> ProjectRoot {
        ProjectRoot::try_new(PathBuf::from("/repo")).unwrap()
    }

    /// Setting then listing shows `key  value`; getting returns the value.
    #[test]
    fn set_then_get_and_list() {
        let store = MemoryCoordination::default();
        set(&store, &project(), &Author::human(), "port", "3000").unwrap();
        set(&store, &project(), &Author::human(), "port", "4000").unwrap();

        let listed = list(&store, &project(), PageSize::default()).unwrap();
        assert_eq!(
            listed.rows().len(),
            1,
            "the same key replaces, not duplicates"
        );
        assert_eq!(listed.rows()[0].detail(), "port  4000");
        assert_eq!(listed.json()["entries"][0]["value"], "4000");

        let value = get(&store, &project(), "port").unwrap();
        assert_eq!(value.rows()[0].detail(), "4000");
        assert_eq!(value.json()["key"], "port");
    }

    /// A listing cuts a long value and marks it, pages by `limit` and says
    /// when more follow; `get` still returns the value whole.
    #[test]
    fn list_cuts_values_and_pages() {
        let store = MemoryCoordination::default();
        let long = "x".repeat(LISTED_VALUE_CHARS + 1);
        set(&store, &project(), &Author::human(), "blob", &long).unwrap();
        set(&store, &project(), &Author::human(), "port", "4000").unwrap();

        let first = list(&store, &project(), PageSize::try_new(1).unwrap()).unwrap();
        assert_eq!(first.json()["entries"].as_array().unwrap().len(), 1);
        assert_eq!(first.json()["entries"][0]["truncated"], true);
        assert_eq!(first.json()["more"], true);
        assert!(first.rows()[0].detail().contains(CUT_MARK));
        assert_eq!(first.rows()[1].detail(), MORE_NOTE);

        let all = list(&store, &project(), PageSize::default()).unwrap();
        assert_eq!(all.json()["more"], false);
        assert_eq!(
            get(&store, &project(), "blob").unwrap().json()["value"],
            long
        );
    }

    /// A value past the maximum is refused with its length, and not stored.
    #[test]
    fn an_oversized_value_is_refused() {
        let store = MemoryCoordination::default();
        let value = "x".repeat(KV_VALUE_MAX_CHARS + 1);
        assert!(matches!(
            set(&store, &project(), &Author::human(), "blob", &value),
            Err(CliError::TooLong {
                max: KV_VALUE_MAX_CHARS,
                ..
            })
        ));
        assert!(get(&store, &project(), "blob").is_err());
    }

    /// Getting an absent value errors so scripts see a non-zero exit.
    #[test]
    fn get_absent_is_an_error() {
        let store = MemoryCoordination::default();
        assert!(matches!(
            get(&store, &project(), "ghost"),
            Err(CliError::UnknownValue(key)) if key == "ghost"
        ));
    }

    /// Removing succeeds once, then reports absence idempotently.
    #[test]
    fn remove_is_idempotent() {
        let store = MemoryCoordination::default();
        set(&store, &project(), &Author::human(), "flag", "on").unwrap();
        assert_eq!(
            remove(&store, &project(), "flag").unwrap().rows()[0].kind(),
            RowKind::Ok
        );
        assert_eq!(
            remove(&store, &project(), "flag").unwrap().rows()[0].kind(),
            RowKind::Hint
        );
    }

    /// A blank key is rejected before the store is touched.
    #[test]
    fn a_blank_key_is_rejected() {
        let store = MemoryCoordination::default();
        assert!(matches!(
            set(&store, &project(), &Author::human(), "   ", "v"),
            Err(CliError::InvalidKvKey(_))
        ));
    }

    /// An empty store lists a hint rather than nothing.
    #[test]
    fn list_reports_empty() {
        let store = MemoryCoordination::default();
        assert_eq!(
            list(&store, &project(), PageSize::default())
                .unwrap()
                .rows()[0]
                .detail(),
            EMPTY_NOTE
        );
    }
}
