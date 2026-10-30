use std::thread;

use crossbeam_channel::{Sender, bounded};
use notify_rust::Notification as OsNotification;

use crate::{
    constants::APP_NAME,
    domain::{notification::Notification, port::Notifier},
};

/// Bounded backlog of pending desktop notifications. If the daemon stalls this
/// fills and further notifications are dropped, rather than blocking the
/// caller or growing memory without bound.
const QUEUE_CAPACITY: usize = 64;
/// XDG markup entity for a literal ampersand in a plain-text body.
#[cfg(not(target_os = "macos"))]
const XDG_AMPERSAND: &str = "&amp;";
/// XDG markup entity for a literal less-than sign in a plain-text body.
#[cfg(not(target_os = "macos"))]
const XDG_LESS_THAN: &str = "&lt;";
/// XDG markup entity for a literal greater-than sign in a plain-text body.
#[cfg(not(target_os = "macos"))]
const XDG_GREATER_THAN: &str = "&gt;";

/// A [`Notifier`] that raises OS desktop notifications through `notify-rust`
/// (libnotify/D-Bus on Linux). Delivery runs on a dedicated worker thread, so a
/// slow or unresponsive daemon never blocks the runtime loop, and every error is
/// ignored: notifications are best effort.
pub struct DesktopNotifier {
    sender: Sender<Notification>,
}

impl DesktopNotifier {
    /// Spawns the delivery worker and returns a notifier that feeds it.
    pub fn new() -> Self {
        let (sender, receiver) = bounded::<Notification>(QUEUE_CAPACITY);
        thread::spawn(move || {
            // Ends when the sender (and so this notifier) is dropped on shutdown.
            for notification in receiver {
                let _ = os_notification(&notification).show();
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
        let _ = self.sender.try_send(notification.clone());
    }
}

/// Builds one OS notification from the domain value: its title, or the
/// process's name when it has none, over its plain-text body.
fn os_notification(notification: &Notification) -> OsNotification {
    let summary = notification
        .title()
        .clone()
        .unwrap_or_else(|| notification.source().as_ref().to_string());
    let body = notification
        .body()
        .as_deref()
        .map(notification_body)
        .unwrap_or_default();
    let mut output = OsNotification::new();
    output.appname(APP_NAME).summary(&summary).body(&body);
    output
}

/// Converts a plain-text body into safe XDG notification markup so servers
/// display markup characters literally.
#[cfg(not(target_os = "macos"))]
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

/// Keeps plain text unchanged on backends whose body is not interpreted as XDG
/// markup.
#[cfg(target_os = "macos")]
fn notification_body(body: &str) -> String {
    body.to_string()
}

#[cfg(all(test, not(target_os = "macos")))]
mod tests {
    use super::*;

    #[test]
    fn xdg_delivery_escapes_plain_text_markup_characters() {
        assert_eq!(
            notification_body("<b>two & three</b>"),
            "&lt;b&gt;two &amp; three&lt;/b&gt;"
        );
    }
}
