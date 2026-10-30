use std::{env, path::PathBuf};

use crate::constants::APP_NAME;

/// This muster binary as it is now, for what runs it again, or its name
/// when the path cannot be read. On Linux a binary replaced or removed
/// since it started reads as a path that no longer exists.
pub fn this_binary() -> PathBuf {
    env::current_exe().unwrap_or_else(|_| PathBuf::from(APP_NAME))
}
