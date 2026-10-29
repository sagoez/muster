use crate::domain::{
    config::ConfigError,
    coordination::{
        Author, Claim, EntryPage, EntryPosition, KeyValue, KvKey, KvValue, Lock, LockAttempt,
        LockHolder, LockName, LockNote, LockRelease, LockTtl, NewTodoTitle, NoteBody, NoteHeader,
        Scratchpad, ScratchpadKey, SessionId, Todo, TodoChange, TodoDeps, TodoId, TodoLease,
        TodoUpdate,
    },
    value::ProjectRoot,
};

/// Driven port: durable shared coordination state for a project - the
/// scratchpads, todos, and key-values agents and the human read and write to
/// pass context and coordinate work. Each write is atomic and scoped to the entry
/// it touches, so concurrent agents cannot lose each other's updates. Every
/// method fails with [`ConfigError::UnknownProject`] when `project` is not
/// registered.
pub trait CoordinationStore {
    /// Every scratchpad under `project`, in the order each was first written.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the store cannot be read or parsed.
    fn scratchpads(&self, project: &ProjectRoot) -> Result<Vec<Scratchpad>, ConfigError>;

    /// At most `limit` of `project`'s notes without bodies, in the order each
    /// was first written, after the position `after` (from the start without
    /// one). An entry deleted at `after` does not break the listing.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the store cannot be read or parsed.
    fn scratchpad_page(
        &self,
        project: &ProjectRoot,
        after: Option<EntryPosition>,
        limit: usize,
    ) -> Result<EntryPage<NoteHeader>, ConfigError>;

    /// The key of every scratchpad under `project`, in the order each was
    /// first written.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the store cannot be read or parsed.
    fn scratchpad_keys(&self, project: &ProjectRoot) -> Result<Vec<ScratchpadKey>, ConfigError>;

    /// The scratchpad named `key` under `project`, if present.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the store cannot be read or parsed.
    fn scratchpad(
        &self,
        project: &ProjectRoot,
        key: &ScratchpadKey,
    ) -> Result<Option<Scratchpad>, ConfigError>;

    /// Creates or replaces the scratchpad named `key` under `project` with `body`,
    /// stamping `author` and the write time.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the store cannot be read or written.
    fn set_scratchpad(
        &self,
        project: &ProjectRoot,
        author: &Author,
        key: &ScratchpadKey,
        body: &NoteBody,
    ) -> Result<(), ConfigError>;

    /// Removes the scratchpad named `key` under `project`. Returns whether one was
    /// removed.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the store cannot be read or written.
    fn delete_scratchpad(
        &self,
        project: &ProjectRoot,
        key: &ScratchpadKey,
    ) -> Result<bool, ConfigError>;

    /// Every todo under `project`, in creation order.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the store cannot be read or parsed.
    fn todos(&self, project: &ProjectRoot) -> Result<Vec<Todo>, ConfigError>;

    /// Adds a new pending todo under `project` with `title` and `deps`, assigning
    /// a fresh id and stamping `author` and the write time. Returns the created
    /// todo.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if an id cannot be generated or the store
    /// cannot be read or written.
    fn add_todo(
        &self,
        project: &ProjectRoot,
        author: &Author,
        title: &NewTodoTitle,
        deps: &TodoDeps,
    ) -> Result<Todo, ConfigError>;

    /// Applies `change` by `me` to the todo identified by `id` under `project`,
    /// as [`Todo::changed`] rules, in one transaction.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the store cannot be read or written.
    fn update_todo(
        &self,
        project: &ProjectRoot,
        me: &Author,
        id: &TodoId,
        change: &TodoChange,
    ) -> Result<TodoUpdate, ConfigError>;

    /// Atomically claims a todo under `project` for `me` for `lease`: the one
    /// `id` names when it is pending, its lease ran out, or `me` holds it
    /// (renewing); without `id`, the oldest todo
    /// [`claimable`](crate::domain::coordination::claimable) yields.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the store cannot be read or written.
    fn claim_todo(
        &self,
        project: &ProjectRoot,
        me: &Author,
        id: Option<&TodoId>,
        lease: TodoLease,
    ) -> Result<Claim, ConfigError>;

    /// Removes the todo identified by `id` under `project`. Returns whether one
    /// was removed.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the store cannot be read or written.
    fn delete_todo(&self, project: &ProjectRoot, id: &TodoId) -> Result<bool, ConfigError>;

    /// Every key-value entry under `project`, in the order each was first
    /// written.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the store cannot be read or parsed.
    fn values(&self, project: &ProjectRoot) -> Result<Vec<KeyValue>, ConfigError>;

    /// At most `limit` of `project`'s key-value entries, in the order each
    /// was first written, after the position `after` (from the start without
    /// one). An entry deleted at `after` does not break the listing.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the store cannot be read or parsed.
    fn value_page(
        &self,
        project: &ProjectRoot,
        after: Option<EntryPosition>,
        limit: usize,
    ) -> Result<EntryPage<KeyValue>, ConfigError>;

    /// The key of every key-value entry under `project`, in the order each
    /// was first written.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the store cannot be read or parsed.
    fn value_keys(&self, project: &ProjectRoot) -> Result<Vec<KvKey>, ConfigError>;

    /// The key-value entry named `key` under `project`, if present.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the store cannot be read or parsed.
    fn value(&self, project: &ProjectRoot, key: &KvKey) -> Result<Option<KeyValue>, ConfigError>;

    /// Creates or replaces the key-value entry named `key` under `project` with
    /// `value`, stamping `author` and the write time.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the store cannot be read or written.
    fn set_value(
        &self,
        project: &ProjectRoot,
        author: &Author,
        key: &KvKey,
        value: &KvValue,
    ) -> Result<(), ConfigError>;

    /// Removes the key-value entry named `key` under `project`. Returns whether
    /// one was removed.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the store cannot be read or written.
    fn delete_value(&self, project: &ProjectRoot, key: &KvKey) -> Result<bool, ConfigError>;

    /// Takes the lock `name` under `project` for `me` for `ttl`, in one
    /// statement: free or expired, it becomes `me`'s; held by `me`, through
    /// the same session, it renews and takes `note`.
    ///
    /// # Errors
    /// Returns [`ConfigError::TooManyLocks`] when `me`'s session already
    /// holds the most live locks it may, and a [`ConfigError`] if the store
    /// cannot be read or written.
    fn acquire_lock(
        &self,
        project: &ProjectRoot,
        me: &LockHolder,
        name: &LockName,
        ttl: LockTtl,
        note: Option<&LockNote>,
    ) -> Result<LockAttempt, ConfigError>;

    /// Releases the lock `name` under `project` when `me`, through the same
    /// session, holds it.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the store cannot be read or written.
    fn release_lock(
        &self,
        project: &ProjectRoot,
        me: &LockHolder,
        name: &LockName,
    ) -> Result<LockRelease, ConfigError>;

    /// Removes the lock `name` under `project` whoever holds it, for the
    /// human clearing a stuck lock. Returns the lock removed.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the store cannot be read or written.
    fn break_lock(
        &self,
        project: &ProjectRoot,
        name: &LockName,
    ) -> Result<Option<Lock>, ConfigError>;

    /// Ends `session`, as an MCP server does when its connection ends:
    /// releases every lock taken through it in any project, and in the same
    /// write marks it ended, so [`Self::acquire_lock`] through it answers
    /// [`LockAttempt::SessionEnded`] from then on. Returns how many locks went.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the store cannot be written.
    fn release_locks_of(&self, session: &SessionId) -> Result<usize, ConfigError>;

    /// Every lock under `project` that has not expired, by name.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the store cannot be read or parsed.
    fn locks(&self, project: &ProjectRoot) -> Result<Vec<Lock>, ConfigError>;

    /// Restores entries into `project` exactly as given - preserving each entry's
    /// id, author, and timestamp rather than stamping fresh ones - so a snapshot
    /// round-trips faithfully and todo dependencies keep pointing at real ids.
    /// Entries merge: one sharing a key or id replaces what is there, and entries
    /// absent from the snapshot are left alone. All-or-nothing.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the entries cannot be written.
    fn import(
        &self,
        project: &ProjectRoot,
        scratchpads: &[Scratchpad],
        todos: &[Todo],
        values: &[KeyValue],
    ) -> Result<(), ConfigError>;
}
