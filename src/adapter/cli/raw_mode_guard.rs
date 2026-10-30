use std::{
    io,
    sync::{Arc, Mutex, MutexGuard, PoisonError, Weak},
};

use super::{
    held_terminal::HeldTerminal, host_terminal::HostTerminal, session_alarm::on_session_helper,
};

/// Every terminal a guard holds in raw mode, for the panic hook to give back
/// before the panic is printed.
static HELD: Mutex<Vec<Weak<Mutex<HeldTerminal>>>> = Mutex::new(Vec::new());

/// Holds a terminal in raw mode for `muster attach` and draws on it. On drop,
/// which a panic's unwind runs too, it returns the terminal to how it found
/// it: the alternate screen left when it is on it, charset, keyboard, mouse
/// and other modes back to defaults, the cursor shown, the window title
/// restored and raw mode off. [`restore_terminals`] does the same from a
/// panic hook, before the panic message is printed.
pub struct RawModeGuard {
    /// The terminal, shared with the panic hook through [`HELD`].
    terminal: Arc<Mutex<HeldTerminal>>,
}

impl RawModeGuard {
    /// Puts `terminal` in raw mode and saves its window title.
    ///
    /// # Errors
    /// Returns the I/O error when the terminal cannot be read, switched or
    /// written.
    pub fn enter(terminal: &HostTerminal) -> io::Result<Self> {
        let held = Arc::new(Mutex::new(HeldTerminal::enter(terminal)?));
        let mut registry = HELD.lock().unwrap_or_else(PoisonError::into_inner);
        registry.retain(|held| held.strong_count() > 0);
        registry.push(Arc::downgrade(&held));
        Ok(Self { terminal: held })
    }

    /// Draws `bytes`.
    ///
    /// # Errors
    /// Returns the I/O error when the terminal cannot be written.
    pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.held().write(bytes)
    }

    /// Shows `title` as the window title until the terminal is given back.
    ///
    /// # Errors
    /// Returns the I/O error when the terminal cannot be written.
    pub fn set_title(&mut self, title: &str) -> io::Result<()> {
        self.held().set_title(title)
    }

    /// Readies the terminal for a process's first screen, keeping what was
    /// on display in the terminal's history.
    ///
    /// # Errors
    /// Returns the I/O error when the terminal cannot be written.
    pub fn clear_for_first_screen(&mut self, rows: u16) -> io::Result<()> {
        self.held().clear_for_first_screen(rows)
    }

    /// Readies the terminal for a process's screen sent again.
    ///
    /// # Errors
    /// Returns the I/O error when the terminal cannot be written.
    pub fn clear_for_resync(&mut self) -> io::Result<()> {
        self.held().clear_for_resync()
    }

    /// The held terminal, locked.
    fn held(&self) -> MutexGuard<'_, HeldTerminal> {
        self.terminal.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for RawModeGuard {
    /// Restores the terminal; a terminal that has gone cannot be helped.
    fn drop(&mut self) {
        let _ = self.held().restore();
    }
}

/// Gives back every terminal a [`RawModeGuard`] still holds, for a panic
/// hook to run before the panic is printed. A terminal locked by the
/// panicking code is left to its guard's drop.
/// A panic on a session's helper thread is left to the session, which ends
/// and gives the terminal back itself.
pub fn restore_terminals() {
    if on_session_helper() {
        return;
    }
    if let Ok(registry) = HELD.try_lock() {
        restore_held(&registry);
    }
}

/// Gives back each of `held` still alive and not locked.
fn restore_held(held: &[Weak<Mutex<HeldTerminal>>]) {
    for held in held.iter().filter_map(Weak::upgrade) {
        if let Ok(mut terminal) = held.try_lock() {
            let _ = terminal.restore();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        panic::{self, AssertUnwindSafe},
        thread,
    };

    use super::*;
    use crate::adapter::cli::{
        held_terminal::{
            KITTY_KEYBOARD_RESET, LEAVE_ALT_SCREEN, MODES_RESET, ORIGIN_MODE_OFF, POP_TITLE,
            PUSH_TITLE, SESSION_END,
        },
        session_alarm::{SessionAlarm, mark_session_helper},
        test_pty::{TestPty, contains, same_settings},
    };

    /// Rows of the test terminal.
    const ROWS: u16 = 24;
    /// Columns of the test terminal.
    const COLS: u16 = 80;
    /// What a process draws to enter the alternate screen.
    const ENTER_ALT_SCREEN: &[u8] = b"\x1b[?1049h";
    /// What a process draws to switch to the line-drawing charset.
    const LINE_DRAWING: &[u8] = b"\x1b(0";
    /// What a process draws to switch origin mode on.
    const ORIGIN_MODE_ON: &[u8] = b"\x1b[?6h";

    /// The tail every restore draws, whatever the process left on.
    fn restore_tail() -> Vec<u8> {
        let mut tail = MODES_RESET.concat();
        tail.push_str(SESSION_END);
        tail.push_str(POP_TITLE);
        tail.into_bytes()
    }

    /// How many times `needle` was drawn on `pty`.
    fn count(pty: &TestPty, needle: &[u8]) -> usize {
        pty.drawn()
            .windows(needle.len())
            .filter(|window| *window == needle)
            .count()
    }

    /// Dropping the guard, as a session that ends does and as a panic's
    /// unwind does, turns raw mode off, leaves the alternate screen the
    /// process entered and resets every mode it may have turned on.
    #[test]
    fn the_guard_restores_the_terminal_on_drop_and_on_panic() {
        for panics in [false, true] {
            let pty = TestPty::open(ROWS, COLS);
            let cooked = pty.settings();
            let session = || {
                let mut guard = RawModeGuard::enter(&pty.terminal()).unwrap();
                assert!(!same_settings(&pty.settings(), &cooked), "raw mode is on");
                guard.write(ENTER_ALT_SCREEN).unwrap();
                if panics {
                    panic!("a bug while attached");
                }
            };
            let outcome = panic::catch_unwind(AssertUnwindSafe(session));

            assert_eq!(outcome.is_err(), panics);
            assert!(same_settings(&pty.settings(), &cooked), "raw mode is off");
            let left = [KITTY_KEYBOARD_RESET, LEAVE_ALT_SCREEN].concat();
            let expected = [left.as_bytes(), &restore_tail()].concat();
            pty.wait_for(&expected);
            assert!(contains(&pty.drawn(), PUSH_TITLE.as_bytes()));
        }
    }

    /// A terminal the process left on its primary screen is not switched,
    /// since leaving a screen it is not on would move the cursor, and origin
    /// mode is turned off only when it is on, including when leaving the
    /// alternate screen brings back the origin mode saved on entry.
    #[test]
    fn a_terminal_on_its_primary_screen_is_not_switched_on_restore() {
        let pty = TestPty::open(ROWS, COLS);
        let mut guard = RawModeGuard::enter(&pty.terminal()).unwrap();
        guard.write(ENTER_ALT_SCREEN).unwrap();
        guard.write(b"\x1b[?1049l").unwrap();
        drop(guard);

        pty.wait_for(&restore_tail());
        assert_eq!(
            count(&pty, LEAVE_ALT_SCREEN.as_bytes()),
            1,
            "only the process's own switch back"
        );
        assert_eq!(count(&pty, ORIGIN_MODE_OFF.as_bytes()), 0);

        let pty = TestPty::open(ROWS, COLS);
        let mut guard = RawModeGuard::enter(&pty.terminal()).unwrap();
        guard.write(ORIGIN_MODE_ON).unwrap();
        drop(guard);
        pty.wait_for(&[ORIGIN_MODE_OFF.as_bytes(), &restore_tail()].concat());

        let pty = TestPty::open(ROWS, COLS);
        let mut guard = RawModeGuard::enter(&pty.terminal()).unwrap();
        guard.write(b"\x1b[?6;1049h\x1b[?6l").unwrap();
        drop(guard);
        let left = [KITTY_KEYBOARD_RESET, LEAVE_ALT_SCREEN, ORIGIN_MODE_OFF].concat();
        pty.wait_for(&[left.as_bytes(), &restore_tail()].concat());
    }

    /// A process that switched to the line-drawing charset leaves the
    /// terminal in ASCII once it is given back.
    #[test]
    fn a_line_drawing_charset_is_switched_back_to_ascii() {
        let pty = TestPty::open(ROWS, COLS);
        let mut guard = RawModeGuard::enter(&pty.terminal()).unwrap();
        guard.write(LINE_DRAWING).unwrap();
        drop(guard);

        pty.wait_for(POP_TITLE.as_bytes());
        let drawn = pty.drawn();
        let position = |needle: &[u8]| drawn.windows(needle.len()).rposition(|w| w == needle);
        assert!(
            position(b"\x1b(B\x0f") > position(LINE_DRAWING),
            "ASCII comes last"
        );
    }

    /// The panic hook leaves a live session's terminal alone when the panic
    /// is on one of the session's helper threads.
    #[test]
    fn a_helper_thread_panic_leaves_the_terminal_to_its_session() {
        let pty = TestPty::open(ROWS, COLS);
        let cooked = pty.settings();
        let guard = RawModeGuard::enter(&pty.terminal()).unwrap();

        thread::spawn(|| {
            mark_session_helper(Arc::new(SessionAlarm::default()));
            restore_terminals();
        })
        .join()
        .unwrap();

        assert!(!same_settings(&pty.settings(), &cooked), "still raw");
        drop(guard);
        assert!(same_settings(&pty.settings(), &cooked));
    }

    /// A guard registers its terminal for the panic hook, whose restore
    /// gives it back at once; the guard's drop after it draws nothing more.
    /// Only this test's terminal is restored, since other tests hold theirs.
    #[test]
    fn the_panic_hook_restores_held_terminals_once() {
        let pty = TestPty::open(ROWS, COLS);
        let cooked = pty.settings();
        let mut guard = RawModeGuard::enter(&pty.terminal()).unwrap();
        guard.set_title("muster: api").unwrap();
        let own = Arc::downgrade(&guard.terminal);
        assert!(
            HELD.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .any(|held| held.ptr_eq(&own)),
            "the guard is registered"
        );

        restore_held(&[own]);

        assert!(same_settings(&pty.settings(), &cooked), "raw mode is off");
        pty.wait_for(&restore_tail());
        drop(guard);
        assert_eq!(count(&pty, POP_TITLE.as_bytes()), 1, "restored once");
        assert!(contains(&pty.drawn(), b"\x1b]2;muster: api\x07"));
    }

    /// A title's control characters are drawn escaped, so it can neither end
    /// the title early nor send a sequence of its own.
    #[test]
    fn a_title_is_drawn_printable() {
        let pty = TestPty::open(ROWS, COLS);
        let mut guard = RawModeGuard::enter(&pty.terminal()).unwrap();
        guard.set_title("api\x07\x1b[2J").unwrap();
        pty.wait_for(b"\x1b]2;api\\u{7}\\u{1b}[2J\x07");
        drop(guard);
    }
}
