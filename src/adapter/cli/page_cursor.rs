//! The `--cursor` a `todo`, `note` or `kv` listing resumes from.

use super::error::CliError;
use crate::domain::coordination::EntryPosition;

/// The position an earlier page printed as `raw`.
///
/// # Errors
/// Returns [`CliError::InvalidCursor`] for text no page printed.
pub(super) fn parse_cursor(raw: &str) -> Result<EntryPosition, CliError> {
    raw.trim()
        .parse()
        .map_err(|_| CliError::InvalidCursor(raw.to_string()))
}
