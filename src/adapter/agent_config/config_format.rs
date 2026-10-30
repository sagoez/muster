use super::json_dialect::JsonDialect;

/// How an agent's config file is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ConfigFormat {
    /// JSON, the table of servers under these keys.
    Json(Vec<String>, JsonDialect),
    /// Codex's TOML.
    Toml,
}
