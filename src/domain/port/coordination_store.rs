use crate::domain::{
    config::ConfigError,
    coordination::{
        Author, KeyValue, KvKey, Scratchpad, ScratchpadKey, Todo, TodoId, TodoStatus, TodoTitle,
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
    /// Every scratchpad under `project`, in write order.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the store cannot be read or parsed.
    fn scratchpads(&self, project: &ProjectRoot) -> Result<Vec<Scratchpad>, ConfigError>;

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
        body: &str,
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

    /// The todo identified by `id` under `project`, if present.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the store cannot be read or parsed.
    fn todo(&self, project: &ProjectRoot, id: &TodoId) -> Result<Option<Todo>, ConfigError>;

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
        title: &TodoTitle,
        deps: &[TodoId],
    ) -> Result<Todo, ConfigError>;

    /// Sets the lifecycle status of the todo identified by `id` under `project`,
    /// stamping `author` and the write time. Returns whether the todo was found.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the store cannot be read or written.
    fn set_todo_status(
        &self,
        project: &ProjectRoot,
        author: &Author,
        id: &TodoId,
        status: TodoStatus,
    ) -> Result<bool, ConfigError>;

    /// Removes the todo identified by `id` under `project`. Returns whether one
    /// was removed.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the store cannot be read or written.
    fn delete_todo(&self, project: &ProjectRoot, id: &TodoId) -> Result<bool, ConfigError>;

    /// Every key-value entry under `project`, in write order.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the store cannot be read or parsed.
    fn values(&self, project: &ProjectRoot) -> Result<Vec<KeyValue>, ConfigError>;

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
        value: &str,
    ) -> Result<(), ConfigError>;

    /// Removes the key-value entry named `key` under `project`. Returns whether
    /// one was removed.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the store cannot be read or written.
    fn delete_value(&self, project: &ProjectRoot, key: &KvKey) -> Result<bool, ConfigError>;

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
