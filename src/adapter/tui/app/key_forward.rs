use super::*;

/// Bracketed-paste introducer sent to a child before pasted text (`CSI 200 ~`).
const BRACKETED_PASTE_START: &[u8] = b"\x1b[200~";
/// Bracketed-paste terminator sent to a child after pasted text (`CSI 201 ~`).
const BRACKETED_PASTE_END: &[u8] = b"\x1b[201~";

impl App {
    /// Forwards a key press or repeat to the focused pane's PTY, if it is alive.
    /// Typing snaps a scrolled-back view down to the live screen first. A repeat
    /// of a still-held key follows that press to its original child even if the
    /// selection has since moved, so the child never sees a stray repeat.
    pub(super) fn forward_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Repeat
            && let Some(held) = self.held_terminal_keys.get(&HeldKeyId::of(key)).copied()
        {
            self.write_held(held, key);
            return;
        }
        let Some(pane) = self.selected_pane() else {
            return;
        };
        let (wrote, protocol, screen_epoch) = {
            let Some(target) = self.panes.get_mut(&pane) else {
                return;
            };
            let protocol: KeyboardProtocol = target.keyboard.protocol();
            // A repeat with no held entry (the first branch handled tracked ones)
            // means the matching press never reached this child - it was consumed
            // by a leader command or overlay, or focus moved since. If the key
            // would carry a Kitty event type, forwarding it now emits an unmatched
            // `:2` the child cannot pair with a press, so drop it. Plain text and
            // legacy keys have no event type and still auto-repeat.
            if key.kind == KeyEventKind::Repeat && keyboard::held_encoding(key, protocol).is_some()
            {
                return;
            }
            let screen_epoch = target.keyboard.screen_epoch();
            let wrote = match keyboard::encode_key(key, protocol) {
                Some(bytes) => {
                    target.parser.set_scrollback(0);
                    if let Some(handle) = target.handle.as_mut() {
                        let _ = handle.write_input(&bytes);
                    }
                    true
                },
                None => false,
            };
            (wrote, protocol, screen_epoch)
        };
        // Only a press establishes the release target, and only for a key whose
        // press was escape-coded, so the child will receive a matching release.
        // The stored form lets that release apply its own current modifiers.
        if wrote
            && key.kind == KeyEventKind::Press
            && let Some(encoding) = keyboard::held_encoding(key, protocol)
            && let Some(&generation) = self.generations.get(&pane)
        {
            self.held_terminal_keys
                .insert(HeldKeyId::of(key), HeldPress {
                    pane,
                    generation,
                    encoding,
                    protocol,
                    screen_epoch,
                });
        }
    }

    /// Routes a bracketed paste from the host. Into an open text form it inserts
    /// the characters; attached to a live terminal it forwards the text to the
    /// child, wrapped in bracketed-paste markers when the child requested them so
    /// the child treats it as one paste instead of a burst of typed keys (which
    /// an agent like Codex flags for sanitizing). It is ignored elsewhere.
    pub(super) fn handle_paste(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        if matches!(self.overlay, Some(Overlay::Form(_))) {
            for ch in text.chars().filter(|ch| !ch.is_control()) {
                self.handle_form_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
            }
            return;
        }
        if self.focus != Focus::Terminal {
            return;
        }
        let Some(pane) = self.selected_pane() else {
            return;
        };
        let Some(target) = self.panes.get_mut(&pane) else {
            return;
        };
        // Typing (here, pasting) snaps a scrolled-back view down to the live screen.
        target.parser.set_scrollback(0);
        let bracketed = target.parser.bracketed_paste();
        let Some(handle) = target.handle.as_mut() else {
            return;
        };
        if bracketed {
            let _ = handle.write_input(BRACKETED_PASTE_START);
            let _ = handle.write_input(text.as_bytes());
            let _ = handle.write_input(BRACKETED_PASTE_END);
        } else {
            let _ = handle.write_input(text.as_bytes());
        }
    }

    /// Relays a key release to the child whose matching press was forwarded,
    /// dropping it when no such press is held or that child is no longer running.
    pub(super) fn forward_release(&mut self, key: KeyEvent) {
        let Some(held) = self.held_terminal_keys.remove(&HeldKeyId::of(key)) else {
            return;
        };
        self.write_held(held, key);
    }

    /// Encodes `event` (a repeat or release of a held key) with the key's stored
    /// escape-coded form and writes it to the child that received the press, if
    /// that child is still the live process. The event supplies its own current
    /// modifiers, so a release after the modifier was lifted still matches. The
    /// event is dropped if the child's negotiated protocol or active screen
    /// changed since the press, so a stale Kitty sequence never reaches a child
    /// that has since popped its flags or left the alternate screen - the screen
    /// epoch catches the screen change even when both screens share flags.
    pub(super) fn write_held(&mut self, held: HeldPress, event: KeyEvent) {
        if self.generations.get(&held.pane) != Some(&held.generation) {
            return;
        }
        let Some(target) = self.panes.get_mut(&held.pane) else {
            return;
        };
        if target.keyboard.protocol() != held.protocol
            || target.keyboard.screen_epoch() != held.screen_epoch
        {
            return;
        }
        let bytes = keyboard::encode_held(held.encoding, event);
        if let Some(handle) = target.handle.as_mut() {
            let _ = handle.write_input(&bytes);
        }
    }

    /// Resizes every live pane's PTY and parser to match `area`.
    pub fn resize(&mut self, area: Rect) {
        self.frame_area = area;
        self.pane_size = pane_size_of(area);
        let rows = self.pane_size.rows().into_inner();
        let cols = self.pane_size.cols().into_inner();
        let size = self.pane_size;
        for pane in self.panes.values_mut() {
            pane.parser.set_size(rows, cols);
            if let Some(handle) = pane.handle.as_mut() {
                let _ = handle.resize(size);
            }
        }
    }
}
