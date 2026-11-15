use super::sqlite_store::SqliteStore;

/// What a read-only look at an existing database found.
pub enum ExistingDatabase {
    /// There is no database yet.
    Absent,
    /// An older muster wrote it; the next command that opens it to write
    /// migrates it.
    Older,
    /// It is at the current schema, opened read-only.
    Current(SqliteStore),
}
