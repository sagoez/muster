use super::*;

/// How long an error notice stays up before it auto-dismisses. Longer than a
/// success toast so a failure has time to be read.
const NOTICE_DURATION: Duration = Duration::from_secs(6);

impl App {
    /// Raises a transient toast, replacing any current one; it auto-dismisses
    /// after `TOAST_DURATION`.
    pub(super) fn show_toast(&mut self, message: &str, tone: ToastTone) {
        self.toast = Some(
            Toast::builder()
                .message(message.to_string())
                .expires_at(Instant::now() + TOAST_DURATION)
                .tone(tone)
                .build(),
        );
    }

    /// When the active toast should auto-dismiss, if one is showing.
    pub fn next_toast_deadline(&self) -> Option<Instant> {
        self.toast.as_ref().map(|toast| *toast.expires_at())
    }

    /// Clears the toast once its deadline has passed. Returns whether a redraw
    /// is needed.
    pub fn expire_toast(&mut self, now: Instant) -> bool {
        if self
            .toast
            .as_ref()
            .is_some_and(|toast| *toast.expires_at() <= now)
        {
            self.toast = None;
            return true;
        }
        false
    }

    /// Shows a transient failure notice and arms its auto-dismiss deadline.
    /// Every notice assignment goes through here so a replacement always resets
    /// the deadline instead of inheriting the prior one.
    pub(super) fn set_notice(&mut self, message: String) {
        self.notice = Some(message);
        self.notice_deadline = Some(Instant::now() + NOTICE_DURATION);
    }

    /// When the current notice auto-dismisses, if one is showing.
    pub fn next_notice_deadline(&self) -> Option<Instant> {
        self.notice_deadline
    }

    /// Clears the notice once its auto-dismiss deadline has passed. Returns
    /// whether a redraw is needed.
    pub fn expire_notice(&mut self, now: Instant) -> bool {
        if self.notice_deadline.is_some_and(|deadline| now >= deadline) {
            self.notice = None;
            self.notice_deadline = None;
            return true;
        }
        false
    }

    /// Dismisses the toast whose box contains `position`, mirroring the render
    /// stack (error notice at the bottom, success toast above). Returns whether
    /// a box was dismissed, so the click is consumed rather than reaching the
    /// pane beneath.
    pub(super) fn dismiss_toast_at(&mut self, main_area: Rect, position: Position) -> bool {
        let mut consumed = 0;
        let notice_rect = self
            .notice
            .as_deref()
            .and_then(|notice| toast::region(main_area, notice, ToastTone::Error, consumed));
        if let Some(rect) = notice_rect {
            if rect.contains(position) {
                self.notice = None;
                self.notice_deadline = None;
                return true;
            }
            consumed = rect.height;
        }
        let toast_rect = self
            .toast
            .as_ref()
            .and_then(|toast| toast::region(main_area, toast.message(), *toast.tone(), consumed));
        if let Some(rect) = toast_rect
            && rect.contains(position)
        {
            self.toast = None;
            return true;
        }
        false
    }
}
