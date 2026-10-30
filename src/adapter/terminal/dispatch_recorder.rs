use alacritty_terminal::vte::Perform;

/// Records what a parser dispatched: the printed text and every OSC payload.
#[derive(Default)]
pub(super) struct DispatchRecorder {
    printed: String,
    oscs: Vec<Vec<u8>>,
}

impl DispatchRecorder {
    /// Every character printed, in order.
    pub(super) fn printed(&self) -> &str {
        &self.printed
    }

    /// Every OSC payload dispatched, its parameters joined.
    pub(super) fn oscs(&self) -> &[Vec<u8>] {
        &self.oscs
    }
}

impl Perform for DispatchRecorder {
    fn print(&mut self, character: char) {
        self.printed.push(character);
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        self.oscs.push(params.concat());
    }
}
