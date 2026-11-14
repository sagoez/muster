use alacritty_terminal::vte::{Params, Perform};

/// A performer that passes every call on to the one it wraps and notes that
/// one came, so a parse can tell whether it dispatched anything.
pub(super) struct WatchedPerformer<'a, P: Perform> {
    /// The performer the parse is for.
    inner: &'a mut P,
    /// Whether any call came.
    dispatched: bool,
}

impl<'a, P: Perform> WatchedPerformer<'a, P> {
    /// Watches `inner`, which has heard nothing yet.
    pub(super) fn new(inner: &'a mut P) -> Self {
        Self {
            inner,
            dispatched: false,
        }
    }

    /// Whether any call came.
    pub(super) fn dispatched(&self) -> bool {
        self.dispatched
    }
}

impl<P: Perform> Perform for WatchedPerformer<'_, P> {
    fn print(&mut self, character: char) {
        self.dispatched = true;
        self.inner.print(character);
    }

    fn execute(&mut self, byte: u8) {
        self.dispatched = true;
        self.inner.execute(byte);
    }

    fn hook(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        self.dispatched = true;
        self.inner.hook(params, intermediates, ignore, action);
    }

    fn put(&mut self, byte: u8) {
        self.dispatched = true;
        self.inner.put(byte);
    }

    fn unhook(&mut self) {
        self.dispatched = true;
        self.inner.unhook();
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
        self.dispatched = true;
        self.inner.osc_dispatch(params, bell_terminated);
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        self.dispatched = true;
        self.inner
            .csi_dispatch(params, intermediates, ignore, action);
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        self.dispatched = true;
        self.inner.esc_dispatch(intermediates, ignore, byte);
    }
}
