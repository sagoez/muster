use super::sqlite_store::SqliteStore;

/// What a read-only look at an existing database found. muster-web's
/// `OpenedState` says the same of the state port, since a driver never names
/// this store; the composition root maps this onto it.
#[derive(Debug)]
pub enum ExistingDatabase {
    /// There is no database yet.
    Absent,
    /// An older muster wrote it; the next command that opens it to write
    /// migrates it.
    Older,
    /// It is at the current schema, opened read-only.
    Current(SqliteStore),
}
