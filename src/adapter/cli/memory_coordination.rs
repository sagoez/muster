use std::cell::RefCell;

use crate::domain::{
    config::ConfigError,
    coordination::{
        Author, Claim, EntryPage, EntryPosition, KeyValue, KvKey, KvValue, Lock, LockAttempt,
        LockHolder, LockName, LockNote, LockRelease, LockTtl, NewTodoTitle, NoteBody, NoteHeader,
        Scratchpad, ScratchpadKey, SessionId, Todo, TodoChange, TodoDeps, TodoId, TodoLease,
        TodoUpdate,
    },
    port::CoordinationStore,
    value::{EpochSeconds, ProjectRoot},
};

/// An in-memory coordination store keeping one project's notes and key-value
/// entries; the note and kv commands never reach todos, locks or an import.
#[derive(Default)]
pub(super) struct MemoryCoordination {
    notes: RefCell<Vec<Scratchpad>>,
    entries: RefCell<Vec<KeyValue>>,
}

/// The page of `limit` of `items` after `after`, each item standing at its
/// place in the list counted from one, as `map` lists it.
fn paged<T, U>(
    items: &[T],
    after: Option<EntryPosition>,
    limit: usize,
    map: impl Fn(&T) -> U,
) -> EntryPage<U> {
    let rows = items
        .iter()
        .zip(1_i64..)
        .filter_map(|(item, at)| Some((EntryPosition::try_new(at).ok()?, item)))
        .filter(|(position, _)| after.is_none_or(|after| *position > after))
        .map(|(position, item)| (position, map(item)))
        .take(limit.saturating_add(1))
        .collect();
    EntryPage::of(rows, limit)
}

impl CoordinationStore for MemoryCoordination {
    fn scratchpads(&self, _project: &ProjectRoot) -> Result<Vec<Scratchpad>, ConfigError> {
        Ok(self.notes.borrow().clone())
    }

    fn scratchpad_page(
        &self,
        _project: &ProjectRoot,
        after: Option<EntryPosition>,
        limit: usize,
    ) -> Result<EntryPage<NoteHeader>, ConfigError> {
        Ok(paged(&self.notes.borrow(), after, limit, |note| {
            NoteHeader::builder()
                .key(note.key().clone())
                .author(note.author().clone())
                .updated_at(*note.updated_at())
                .build()
        }))
    }

    fn scratchpad_keys(&self, _project: &ProjectRoot) -> Result<Vec<ScratchpadKey>, ConfigError> {
        Ok(self
            .notes
            .borrow()
            .iter()
            .map(|note| note.key().clone())
            .collect())
    }

    fn scratchpad(
        &self,
        _project: &ProjectRoot,
        key: &ScratchpadKey,
    ) -> Result<Option<Scratchpad>, ConfigError> {
        Ok(self
            .notes
            .borrow()
            .iter()
            .find(|note| note.key() == key)
            .cloned())
    }

    fn set_scratchpad(
        &self,
        _project: &ProjectRoot,
        author: &Author,
        key: &ScratchpadKey,
        body: &NoteBody,
    ) -> Result<(), ConfigError> {
        let note = Scratchpad::builder()
            .key(key.clone())
            .body(body.to_string())
            .author(author.clone())
            .updated_at(EpochSeconds::new(0))
            .build();
        let mut notes = self.notes.borrow_mut();
        notes.retain(|existing| existing.key() != key);
        notes.push(note);
        Ok(())
    }

    fn delete_scratchpad(
        &self,
        _project: &ProjectRoot,
        key: &ScratchpadKey,
    ) -> Result<bool, ConfigError> {
        let mut notes = self.notes.borrow_mut();
        let before = notes.len();
        notes.retain(|note| note.key() != key);
        Ok(notes.len() != before)
    }

    fn todos(&self, _project: &ProjectRoot) -> Result<Vec<Todo>, ConfigError> {
        unreachable!("note and kv commands never touch todos")
    }

    fn positioned_todos(
        &self,
        _project: &ProjectRoot,
    ) -> Result<Vec<(EntryPosition, Todo)>, ConfigError> {
        unreachable!("note and kv commands never touch todos")
    }

    fn add_todo(
        &self,
        _project: &ProjectRoot,
        _author: &Author,
        _title: &NewTodoTitle,
        _deps: &TodoDeps,
    ) -> Result<Todo, ConfigError> {
        unreachable!("note and kv commands never touch todos")
    }

    fn update_todo(
        &self,
        _project: &ProjectRoot,
        _me: &Author,
        _id: &TodoId,
        _change: &TodoChange,
    ) -> Result<TodoUpdate, ConfigError> {
        unreachable!("note and kv commands never touch todos")
    }

    fn claim_todo(
        &self,
        _project: &ProjectRoot,
        _me: &Author,
        _id: Option<&TodoId>,
        _lease: TodoLease,
    ) -> Result<Claim, ConfigError> {
        unreachable!("note and kv commands never touch todos")
    }

    fn delete_todo(&self, _project: &ProjectRoot, _id: &TodoId) -> Result<bool, ConfigError> {
        unreachable!("note and kv commands never touch todos")
    }

    fn acquire_lock(
        &self,
        _project: &ProjectRoot,
        _me: &LockHolder,
        _name: &LockName,
        _ttl: LockTtl,
        _note: Option<&LockNote>,
    ) -> Result<LockAttempt, ConfigError> {
        unreachable!("note and kv commands never touch locks")
    }

    fn release_lock(
        &self,
        _project: &ProjectRoot,
        _me: &LockHolder,
        _name: &LockName,
    ) -> Result<LockRelease, ConfigError> {
        unreachable!("note and kv commands never touch locks")
    }

    fn break_lock(
        &self,
        _project: &ProjectRoot,
        _name: &LockName,
    ) -> Result<Option<Lock>, ConfigError> {
        unreachable!("note and kv commands never touch locks")
    }

    fn release_locks_of(&self, _session: &SessionId) -> Result<usize, ConfigError> {
        unreachable!("note and kv commands never touch locks")
    }

    fn locks(&self, _project: &ProjectRoot) -> Result<Vec<Lock>, ConfigError> {
        unreachable!("note and kv commands never touch locks")
    }

    fn values(&self, _project: &ProjectRoot) -> Result<Vec<KeyValue>, ConfigError> {
        Ok(self.entries.borrow().clone())
    }

    fn value_page(
        &self,
        _project: &ProjectRoot,
        after: Option<EntryPosition>,
        limit: usize,
    ) -> Result<EntryPage<KeyValue>, ConfigError> {
        Ok(paged(&self.entries.borrow(), after, limit, Clone::clone))
    }

    fn value_keys(&self, _project: &ProjectRoot) -> Result<Vec<KvKey>, ConfigError> {
        Ok(self
            .entries
            .borrow()
            .iter()
            .map(|entry| entry.key().clone())
            .collect())
    }

    fn value(&self, _project: &ProjectRoot, key: &KvKey) -> Result<Option<KeyValue>, ConfigError> {
        Ok(self
            .entries
            .borrow()
            .iter()
            .find(|entry| entry.key() == key)
            .cloned())
    }

    fn set_value(
        &self,
        _project: &ProjectRoot,
        author: &Author,
        key: &KvKey,
        value: &KvValue,
    ) -> Result<(), ConfigError> {
        let entry = KeyValue::builder()
            .key(key.clone())
            .value(value.to_string())
            .author(author.clone())
            .updated_at(EpochSeconds::new(0))
            .build();
        let mut entries = self.entries.borrow_mut();
        entries.retain(|existing| existing.key() != key);
        entries.push(entry);
        Ok(())
    }

    fn delete_value(&self, _project: &ProjectRoot, key: &KvKey) -> Result<bool, ConfigError> {
        let mut entries = self.entries.borrow_mut();
        let before = entries.len();
        entries.retain(|entry| entry.key() != key);
        Ok(entries.len() != before)
    }

    fn import(
        &self,
        _project: &ProjectRoot,
        _scratchpads: &[Scratchpad],
        _todos: &[Todo],
        _values: &[KeyValue],
    ) -> Result<(), ConfigError> {
        unreachable!("note and kv commands never import")
    }
}
