use std::{collections::HashMap, thread};

use crossbeam_channel::{Sender, bounded};
use notify_rust::Notification as OsNotification;
#[cfg(all(unix, not(target_os = "macos")))]
use notify_rust::NotificationHandle as OsNotificationHandle;

use crate::{
    constants::APP_NAME,
    domain::{
        notification::{Notification, NotificationId, NotificationScope},
        port::Notifier,
        value::PaneId,
    },
};

/// Bounded backlog of pending desktop notification operations. If the daemon
/// stalls this fills and further operations are dropped, rather than blocking
/// the caller or growing memory without bound.
const QUEUE_CAPACITY: usize = 64;
/// Maximum identified desktop notifications whose delivery handles are kept for
/// later Kitty update or close operations.
const MAX_ACTIVE_NOTIFICATIONS: usize = 64;
/// XDG markup entity for a literal ampersand in a plain-text body.
#[cfg(all(unix, not(target_os = "macos")))]
const XDG_AMPERSAND: &str = "&amp;";
/// XDG markup entity for a literal less-than sign in a plain-text body.
#[cfg(all(unix, not(target_os = "macos")))]
const XDG_LESS_THAN: &str = "&lt;";
/// XDG markup entity for a literal greater-than sign in a plain-text body.
#[cfg(all(unix, not(target_os = "macos")))]
const XDG_GREATER_THAN: &str = "&gt;";

/// A Kitty identifier scoped to the managed pane and terminal lifetime that
/// emitted it.
type NotificationKey = (PaneId, NotificationScope, NotificationId);

/// One operation queued for the desktop-notification worker.
#[derive(Clone)]
enum Delivery {
    /// Shows a new notification or updates the matching identified one.
    Show(Notification),
    /// Closes the matching identified notification if it is still retained.
    Close {
        /// Pane whose terminal owns the identifier.
        pane: PaneId,
        /// Specific terminal lifetime within the reusable pane.
        scope: NotificationScope,
        /// Kitty identifier to close within that pane.
        identifier: NotificationId,
    },
}

/// A [`Notifier`] that raises OS desktop notifications through `notify-rust`
/// (libnotify/D-Bus on Linux). Delivery runs on a dedicated worker thread, so a
/// slow or unresponsive daemon never blocks the runtime loop, and every error is
/// ignored: notifications are best effort.
pub struct DesktopNotifier {
    sender: Sender<Delivery>,
}

impl DesktopNotifier {
    /// Spawns the delivery worker and returns a notifier that feeds it.
    pub fn new() -> Self {
        let (sender, receiver) = bounded::<Delivery>(QUEUE_CAPACITY);
        thread::spawn(move || {
            let mut worker = DeliveryWorker::new(OsBackend);
            // Ends when the sender (and so this notifier) is dropped on shutdown.
            for delivery in receiver {
                worker.deliver(delivery);
            }
        });
        Self { sender }
    }
}

impl Default for DesktopNotifier {
    fn default() -> Self {
        Self::new()
    }
}

impl Notifier for DesktopNotifier {
    fn notify(&self, notification: &Notification) {
        // Hand off to the worker without blocking; drop it if the queue is full.
        let _ = self.sender.try_send(Delivery::Show(notification.clone()));
    }

    fn close(&self, pane: PaneId, scope: NotificationScope, identifier: &NotificationId) {
        let _ = self.sender.try_send(Delivery::Close {
            pane,
            scope,
            identifier: identifier.clone(),
        });
    }
}

/// OS-delivery behavior used by the stateful worker and replaced by a fake in
/// tests so update and close semantics do not require a notification daemon.
trait NotificationBackend {
    /// Retained handle returned when a notification is shown.
    type Handle;

    /// Shows `notification`, returning its handle on success.
    fn show(&mut self, notification: &Notification) -> Option<Self::Handle>;

    /// Replaces the notification represented by `handle` with new content.
    fn update(&mut self, handle: &mut Self::Handle, notification: &Notification) -> bool;

    /// Closes the notification represented by `handle`.
    fn close(&mut self, handle: Self::Handle);
}

/// Owns retained handles and applies queued notification operations in order.
struct DeliveryWorker<B: NotificationBackend> {
    backend: B,
    active: HashMap<NotificationKey, B::Handle>,
}

impl<B: NotificationBackend> DeliveryWorker<B> {
    /// Creates a worker with no retained notification handles.
    fn new(backend: B) -> Self {
        Self {
            backend,
            active: HashMap::new(),
        }
    }

    /// Applies one show/update/close operation.
    fn deliver(&mut self, delivery: Delivery) {
        match delivery {
            Delivery::Show(notification) => self.show(&notification),
            Delivery::Close {
                pane,
                scope,
                identifier,
            } => {
                if let Some(handle) = self.active.remove(&(pane, scope, identifier)) {
                    self.backend.close(handle);
                }
            },
        }
    }

    /// Shows an unidentified notification independently, or updates/replaces the
    /// retained handle for an identified Kitty notification.
    fn show(&mut self, notification: &Notification) {
        let key = notification
            .identifier()
            .clone()
            .map(|identifier| (*notification.pane(), *notification.scope(), identifier));
        if let Some(key) = &key {
            let updated = self
                .active
                .get_mut(key)
                .is_some_and(|handle| self.backend.update(handle, notification));
            if updated {
                return;
            }
            if let Some(handle) = self.active.remove(key) {
                self.backend.close(handle);
            }
        }

        let Some(handle) = self.backend.show(notification) else {
            return;
        };
        let Some(key) = key else {
            return;
        };
        self.evict_if_full();
        self.active.insert(key, handle);
    }

    /// Closes and removes one retained notification before inserting beyond the
    /// active-handle cap.
    fn evict_if_full(&mut self) {
        if self.active.len() < MAX_ACTIVE_NOTIFICATIONS {
            return;
        }
        let key = self.active.keys().next().cloned();
        if let Some(key) = key
            && let Some(handle) = self.active.remove(&key)
        {
            self.backend.close(handle);
        }
    }
}

/// Concrete `notify-rust` backend used by the desktop worker.
struct OsBackend;

#[cfg(all(unix, not(target_os = "macos")))]
impl NotificationBackend for OsBackend {
    type Handle = OsNotificationHandle;

    fn show(&mut self, notification: &Notification) -> Option<Self::Handle> {
        os_notification(notification).show().ok()
    }

    fn update(&mut self, handle: &mut Self::Handle, notification: &Notification) -> bool {
        configure_os_notification(handle, notification);
        handle.update().is_ok()
    }

    fn close(&mut self, handle: Self::Handle) {
        handle.close();
    }
}

// notify-rust's default macOS backend and Windows backend do not expose the
// update/close handle API. They still receive ordered shows; retaining unit
// handles prevents duplicate shows from being treated as unidentified.
#[cfg(not(all(unix, not(target_os = "macos"))))]
impl NotificationBackend for OsBackend {
    type Handle = ();

    fn show(&mut self, notification: &Notification) -> Option<Self::Handle> {
        os_notification(notification).show().ok().map(|_| ())
    }

    fn update(&mut self, _handle: &mut Self::Handle, notification: &Notification) -> bool {
        self.show(notification).is_some()
    }

    fn close(&mut self, _handle: Self::Handle) {}
}

/// Builds one OS notification from the domain value.
fn os_notification(notification: &Notification) -> OsNotification {
    let mut output = OsNotification::new();
    configure_os_notification(&mut output, notification);
    output
}

/// Applies the user-visible notification fields to a notify-rust value or
/// retained handle before its initial show or update.
fn configure_os_notification(output: &mut OsNotification, notification: &Notification) {
    let summary = notification
        .title()
        .clone()
        .unwrap_or_else(|| notification.source().as_ref().to_string());
    let body = notification
        .body()
        .as_deref()
        .map(notification_body)
        .unwrap_or_default();
    output.appname(APP_NAME).summary(&summary).body(&body);
}

/// Converts a protocol-defined plain-text body into safe XDG notification
/// markup so servers display markup characters literally.
#[cfg(all(unix, not(target_os = "macos")))]
fn notification_body(body: &str) -> String {
    let mut escaped = String::with_capacity(body.len());
    for character in body.chars() {
        match character {
            '&' => escaped.push_str(XDG_AMPERSAND),
            '<' => escaped.push_str(XDG_LESS_THAN),
            '>' => escaped.push_str(XDG_GREATER_THAN),
            _ => escaped.push(character),
        }
    }
    escaped
}

/// Keeps protocol-defined plain text unchanged on backends whose body is not
/// interpreted as XDG markup.
#[cfg(not(all(unix, not(target_os = "macos"))))]
fn notification_body(body: &str) -> String {
    body.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::value::ProcessName;

    /// Backend that records operations without contacting a desktop daemon.
    #[derive(Default)]
    struct RecordingBackend {
        shown: Vec<Option<String>>,
        updated: Vec<(usize, Option<String>)>,
        closed: Vec<usize>,
    }

    impl NotificationBackend for RecordingBackend {
        type Handle = usize;

        fn show(&mut self, notification: &Notification) -> Option<Self::Handle> {
            let handle = self.shown.len();
            self.shown.push(notification.body().clone());
            Some(handle)
        }

        fn update(&mut self, handle: &mut Self::Handle, notification: &Notification) -> bool {
            self.updated.push((*handle, notification.body().clone()));
            true
        }

        fn close(&mut self, handle: Self::Handle) {
            self.closed.push(handle);
        }
    }

    /// Builds an identified fixture notification.
    fn notification(
        pane: PaneId,
        scope: NotificationScope,
        identifier: &NotificationId,
        body: &str,
    ) -> Notification {
        Notification::builder()
            .pane(pane)
            .scope(scope)
            .source(ProcessName::try_new("worker").unwrap())
            .body(Some(body.to_string()))
            .identifier(Some(identifier.clone()))
            .build()
    }

    #[test]
    fn a_reused_kitty_identifier_updates_then_closes_one_handle() {
        let pane = PaneId::new(1);
        let scope = NotificationScope::new(1);
        let identifier = NotificationId::try_new("build").unwrap();
        let mut worker = DeliveryWorker::new(RecordingBackend::default());

        worker.deliver(Delivery::Show(notification(
            pane,
            scope,
            &identifier,
            "starting",
        )));
        worker.deliver(Delivery::Show(notification(
            pane,
            scope,
            &identifier,
            "finished",
        )));
        worker.deliver(Delivery::Close {
            pane,
            scope,
            identifier,
        });

        assert_eq!(worker.backend.shown, [Some("starting".to_string())]);
        assert_eq!(worker.backend.updated, [(0, Some("finished".to_string()))]);
        assert_eq!(worker.backend.closed, [0]);
        assert!(worker.active.is_empty());
    }

    #[test]
    fn identical_identifiers_from_different_panes_keep_separate_handles() {
        let first_pane = PaneId::new(1);
        let second_pane = PaneId::new(2);
        let scope = NotificationScope::new(1);
        let identifier = NotificationId::try_new("build").unwrap();
        let mut worker = DeliveryWorker::new(RecordingBackend::default());

        worker.deliver(Delivery::Show(notification(
            first_pane,
            scope,
            &identifier,
            "first",
        )));
        worker.deliver(Delivery::Show(notification(
            second_pane,
            scope,
            &identifier,
            "second",
        )));
        worker.deliver(Delivery::Show(notification(
            first_pane,
            scope,
            &identifier,
            "first update",
        )));
        worker.deliver(Delivery::Close {
            pane: first_pane,
            scope,
            identifier: identifier.clone(),
        });

        assert_eq!(worker.backend.shown, [
            Some("first".to_string()),
            Some("second".to_string())
        ]);
        assert_eq!(worker.backend.updated, [(
            0,
            Some("first update".to_string())
        )]);
        assert_eq!(worker.backend.closed, [0]);
        assert!(
            worker
                .active
                .contains_key(&(second_pane, scope, identifier))
        );
    }

    #[test]
    fn a_reused_pane_identifier_is_independent_in_a_new_terminal_scope() {
        let pane = PaneId::new(1);
        let old_scope = NotificationScope::new(1);
        let new_scope = NotificationScope::new(2);
        let identifier = NotificationId::try_new("build").unwrap();
        let mut worker = DeliveryWorker::new(RecordingBackend::default());

        worker.deliver(Delivery::Show(notification(
            pane,
            old_scope,
            &identifier,
            "old project",
        )));
        worker.deliver(Delivery::Show(notification(
            pane,
            new_scope,
            &identifier,
            "new project",
        )));
        worker.deliver(Delivery::Close {
            pane,
            scope: new_scope,
            identifier: identifier.clone(),
        });

        assert_eq!(worker.backend.shown, [
            Some("old project".to_string()),
            Some("new project".to_string())
        ]);
        assert!(worker.backend.updated.is_empty());
        assert_eq!(worker.backend.closed, [1]);
        assert!(worker.active.contains_key(&(pane, old_scope, identifier)));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn xdg_delivery_escapes_plain_text_markup_characters() {
        assert_eq!(
            notification_body("<b>two & three</b>"),
            "&lt;b&gt;two &amp; three&lt;/b&gt;"
        );
    }
}
