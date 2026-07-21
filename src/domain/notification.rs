use getset::Getters;
use nutype::nutype;
use typed_builder::TypedBuilder;

use crate::domain::value::{PaneId, ProcessName};

/// A Kitty notification identifier used to replace or close a prior desktop
/// notification. The protocol restricts identifiers to escape-safe characters.
#[nutype(
    validate(
        not_empty,
        predicate = |identifier: &str| identifier.bytes().all(|byte| byte.is_ascii_alphanumeric()
            || matches!(byte, b'_' | b'-' | b'+' | b'.'))
    ),
    derive(Debug, Clone, PartialEq, Eq, Hash, AsRef, Display)
)]
pub struct NotificationId(String);

/// Opaque identity for one terminal lifetime. A reused pane starts a new scope,
/// so its Kitty identifiers cannot affect notifications from the prior terminal.
#[nutype(derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display))]
pub struct NotificationScope(u64);

impl NotificationScope {
    /// Returns the next terminal-lifetime identity without exposing its storage
    /// representation to adapters.
    pub fn next(self) -> Self {
        Self::new(self.into_inner().wrapping_add(1))
    }
}

/// A user-facing notification raised from a process's terminal signals: a bell,
/// or an OSC notification sequence (iTerm2 `9`, kitty `99`, rxvt `777`).
#[derive(Clone, Debug, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Notification {
    /// Pane whose terminal emitted the notification.
    pane: PaneId,
    /// Terminal lifetime within which Kitty identifiers are unique.
    scope: NotificationScope,
    /// The process that raised it, used as the notification's context.
    source: ProcessName,
    /// Optional summary line; delivery falls back to `source` when absent.
    #[builder(default)]
    title: Option<String>,
    /// Optional notification message; title-only protocol notifications leave
    /// this absent so desktop delivery does not duplicate the summary.
    #[builder(default)]
    body: Option<String>,
    /// Kitty identifier used to update or close this notification, when present.
    #[builder(default)]
    identifier: Option<NotificationId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kitty_identifiers_accept_only_protocol_safe_characters() {
        assert!(NotificationId::try_new("build_1-done+now.").is_ok());
        assert!(NotificationId::try_new("").is_err());
        assert!(NotificationId::try_new("unsafe/id").is_err());
    }

    #[test]
    fn notification_scopes_advance_without_raw_integer_arithmetic() {
        assert_eq!(NotificationScope::new(0).next(), NotificationScope::new(1));
    }
}
