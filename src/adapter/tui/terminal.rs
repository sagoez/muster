use std::{
    io::{self, Stdout},
    sync::atomic::{AtomicBool, Ordering},
};

use crossterm::{
    clipboard::CopyToClipboard,
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    style::Print,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
        supports_keyboard_enhancement,
    },
};
use getset::MutGetters;
use ratatui::{Terminal, backend::CrosstermBackend};

use super::pointer_shape::PointerShape;
use crate::{adapter::clipboard, error::Result};

/// OSC prefix that sets the host pointer shape (xterm OSC 22).
const POINTER_SHAPE_PREFIX: &str = "\x1b]22;";
/// String terminator closing an OSC sequence.
const OSC_TERMINATOR: &str = "\x1b\\";
/// Keyboard-enhancement level muster asks the host terminal for. Escape-code
/// disambiguation makes the host report a modified special key as a `CSI u`
/// sequence - Shift+Enter arrives as `CSI 13;2u`, which crossterm decodes as
/// Enter with Shift - so muster can tell it apart from a bare Enter's `\r`; it is
/// also what lets crossterm read the Super/Hyper/Meta modifiers at all. Event-type
/// reporting delivers key repeats and releases so muster can forward them to
/// children that enable the Kitty event-type flag.
///
/// `REPORT_ALL_KEYS_AS_ESCAPE_CODES` is deliberately excluded. Per crossterm it is
/// "required to get repeat/release events for plain-text keys": it routes ordinary
/// typing through escape codes, which breaks input-method editors (CJK and
/// similar), the same reason herdr's IME-compatible flag set omits it. It is not
/// what disambiguates Shift+Enter or other modified special keys - disambiguation
/// already does that, and crossterm decodes the result - it only escape-codes
/// unmodified Enter/Tab/Backspace and plain text so their releases can be reported,
/// which the agent CLIs muster runs never require. Correct text entry outranks it.
/// Alternate-key reporting is omitted too: muster emits no layout-alternate
/// codepoints, so requesting them would gain nothing.
const KEYBOARD_ENHANCEMENT: KeyboardEnhancementFlags =
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        .union(KeyboardEnhancementFlags::REPORT_EVENT_TYPES);

/// Set while muster has pushed keyboard-enhancement flags onto the host, so
/// `restore` (including the panic hook) knows to pop exactly once.
static KEYBOARD_ENHANCED: AtomicBool = AtomicBool::new(false);

/// Whether the host terminal accepted muster's keyboard-enhancement request.
/// When false, crossterm delivers only legacy key events, so muster cannot
/// observe Shift+Enter or key releases and must not advertise the Kitty
/// protocol to its children.
pub fn keyboard_enhancement_active() -> bool {
    KEYBOARD_ENHANCED.load(Ordering::SeqCst)
}

/// The concrete ratatui terminal type: a crossterm backend on stdout.
pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// RAII guard that enters raw mode + the alternate screen on construction and
/// restores the original terminal state on drop.
#[derive(MutGetters)]
pub struct TerminalGuard {
    #[getset(get_mut = "pub")]
    terminal: Tui,
}

impl TerminalGuard {
    /// Enters raw mode and the alternate screen, and captures the mouse so
    /// Muster owns pane-scoped selection and scrolling.
    ///
    /// # Errors
    /// Returns an error if raw mode cannot be enabled, the alternate screen
    /// cannot be entered, or the terminal backend fails to initialize.
    pub fn new() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(
            stdout,
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste
        ) {
            let _ = Self::restore();
            return Err(error.into());
        }
        Self::enable_keyboard_enhancement(&mut stdout);
        match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => Ok(Self { terminal }),
            Err(error) => {
                let _ = Self::restore();
                Err(error.into())
            },
        }
    }

    /// Copies `text` onto the system clipboard: a native clipboard tool when
    /// one works (herdr's preference), falling back to OSC 52 through the
    /// host terminal.
    ///
    /// # Errors
    /// Returns an error when the escape sequence cannot be written to stdout.
    pub fn copy_to_clipboard(&mut self, text: &str) -> io::Result<()> {
        if clipboard::write_native(text) {
            return Ok(());
        }
        execute!(io::stdout(), CopyToClipboard::to_clipboard_from(text))
    }

    /// Asks the host terminal to show `shape` as the mouse pointer (OSC 22).
    ///
    /// # Errors
    /// Returns an error when the escape sequence cannot be written to stdout.
    pub fn set_pointer_shape(&mut self, shape: PointerShape) -> io::Result<()> {
        execute!(
            io::stdout(),
            Print(format!("{POINTER_SHAPE_PREFIX}{shape}{OSC_TERMINATOR}"))
        )
    }

    /// Asks the host terminal to disambiguate escape codes when it supports the
    /// Kitty keyboard protocol, so keys like Shift+Enter arrive distinctly from
    /// their legacy encodings. A no-op on terminals without support; records
    /// that a matching pop is owed on restore.
    ///
    /// This marks the host fully enhanced without confirming that both pushed flags
    /// (disambiguate + event types) actually took effect. Verifying that would mean
    /// querying the active flags after the push, but crossterm 0.29 does not expose
    /// its `query_keyboard_enhancement_flags` (it lives behind `pub(crate) mod sys`;
    /// only the boolean `supports_keyboard_enhancement` is public), and the flags
    /// reply arrives as an `InternalEvent` its public `Event` enum never surfaces -
    /// so muster cannot read the applied set without racing crossterm's own input
    /// reader. In practice terminals implement these two foundational flags together
    /// (partial support is a higher-flag concern), so the assumption holds. Revisit
    /// only if crossterm exposes the applied flags; then feed them to the tracker's
    /// `supported` mask so a disambiguate-only host would not advertise releases.
    fn enable_keyboard_enhancement(writer: &mut Stdout) {
        if supports_keyboard_enhancement().unwrap_or(false)
            && execute!(writer, PushKeyboardEnhancementFlags(KEYBOARD_ENHANCEMENT)).is_ok()
        {
            KEYBOARD_ENHANCED.store(true, Ordering::SeqCst);
        }
    }

    /// Pops the keyboard-enhancement flags if muster pushed them, exactly once.
    ///
    /// # Errors
    /// Returns an error if the pop sequence cannot be written to stdout.
    fn disable_keyboard_enhancement() -> io::Result<()> {
        if KEYBOARD_ENHANCED.swap(false, Ordering::SeqCst) {
            execute!(io::stdout(), PopKeyboardEnhancementFlags)
        } else {
            Ok(())
        }
    }

    /// Restores the terminal to its original cooked state. Safe to call more
    /// than once; used by both `Drop` and the panic hook.
    ///
    /// # Errors
    /// Returns an error if raw mode cannot be disabled or the alternate screen
    /// cannot be left.
    pub fn restore() -> io::Result<()> {
        let keyboard = Self::disable_keyboard_enhancement();
        let raw = disable_raw_mode();
        let mouse = execute!(io::stdout(), DisableMouseCapture);
        let paste = execute!(io::stdout(), DisableBracketedPaste);
        let pointer = execute!(
            io::stdout(),
            Print(format!(
                "{POINTER_SHAPE_PREFIX}{}{OSC_TERMINATOR}",
                PointerShape::Default
            ))
        );
        let screen = execute!(io::stdout(), LeaveAlternateScreen);
        keyboard
            .and(raw)
            .and(mouse)
            .and(paste)
            .and(pointer)
            .and(screen)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = Self::restore();
    }
}
