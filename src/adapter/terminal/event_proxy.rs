use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener};

use super::{
    emulator::resolve_color,
    emulator_sink::{EmulatorSink, lock_sink},
};
use crate::domain::color_scheme::ColorScheme;

/// The `alacritty_terminal` event listener. It shares [`EmulatorSink`] with the
/// emulator through `Arc<Mutex<_>>` so the emulator can move between threads.
#[derive(Clone)]
pub(super) struct EventProxy {
    pub(super) sink: Arc<Mutex<EmulatorSink>>,
    pub(super) color_scheme: ColorScheme,
}

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        match event {
            // Status and identity reports the terminal wants written back verbatim.
            Event::PtyWrite(text) => lock_sink(&self.sink).write(text.as_bytes()),
            // A child querying a color (OSC 10/11 for the default foreground and
            // background, OSC 4 for a palette entry) must get an answer, or an agent
            // like Codex, unable to read the background, themes for a light terminal
            // and renders white on white. Reply from a standard palette with the
            // configured scheme's defaults, the way a real terminal does.
            Event::ColorRequest(index, format) => {
                let reply = format(resolve_color(index, self.color_scheme));
                lock_sink(&self.sink).write(reply.as_bytes());
            },
            _ => {},
        }
    }
}
