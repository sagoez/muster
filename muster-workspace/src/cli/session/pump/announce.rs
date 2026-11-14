use std::io;

/// Says the line a session opens with, before it takes the terminal over.
pub type Announce = Box<dyn FnOnce(&str) -> io::Result<()> + Send>;
