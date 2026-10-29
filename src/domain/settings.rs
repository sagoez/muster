use getset::{Getters, WithSetters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use crate::domain::{
    color_scheme::ColorScheme,
    pty::PtySize,
    value::{ByteSize, DetachKey},
};

/// User settings that apply across every workspace, stored per machine rather
/// than in any single project's config.
#[derive(Clone, Debug, Serialize, Deserialize, Getters, WithSetters, TypedBuilder)]
#[set_with]
pub struct Settings {
    /// Whether to raise OS desktop notifications.
    #[getset(get = "pub", set_with = "pub")]
    desktop_notifications: bool,
    /// Terminal size a process starts with, before any client resizes it.
    #[getset(get = "pub", set_with = "pub")]
    default_size: PtySize,
    /// The key that detaches `muster attach` from a process.
    #[getset(get = "pub", set_with = "pub")]
    detach_key: DetachKey,
    /// Size at which a process's output log rotates.
    #[getset(get = "pub", set_with = "pub")]
    log_max_size: ByteSize,
    /// Lines of scrollback each process's terminal keeps.
    #[getset(get = "pub", set_with = "pub")]
    scrollback: usize,
    /// The default colors a process's terminal reports when asked.
    #[getset(get = "pub", set_with = "pub")]
    color_scheme: ColorScheme,
}
