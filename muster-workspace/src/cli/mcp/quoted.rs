//! Shared text as muster shows it to an agent, in the session context and
//! in the guard's refusals: quoted and escaped within a bound, and credited
//! to its writer.

use muster_domain::coordination::Author;

use crate::cli::mcp::context::rendering::Rendering;

/// Follows a quoted text cut short.
pub const SHORTENED: &str = "...";
/// The most bytes an agent's label takes as shown.
pub const LABEL_MAX_BYTES: usize = 80;
/// The most bytes a project's name takes as shown.
pub const PROJECT_NAME_MAX_BYTES: usize = 200;
/// The most bytes a folder takes as shown.
pub const ROOT_MAX_BYTES: usize = 600;
/// Credits an agent, before its label.
const BY: &str = "by";
/// Credits a write made through the CLI, which names no one.
const VIA_CLI: &str = "via muster CLI";

/// `raw` quoted within `max` bytes, or as much of it as fits with
/// [`SHORTENED`] after it, rendered in one pass.
pub fn bounded(raw: &str, max: usize) -> String {
    let (start, cut) = Rendering::Quoted.fitted(raw, max, SHORTENED.len());
    if cut == 0 {
        start
    } else {
        format!("{start}{SHORTENED}")
    }
}

/// How a writer is credited: a write through the CLI, which names no one,
/// as made via muster's CLI, and an agent by its own label, quoted.
pub fn credit(author: &Author) -> String {
    if author.is_human() {
        VIA_CLI.to_owned()
    } else {
        format!("{BY} {}", bounded(author.label(), LABEL_MAX_BYTES))
    }
}
