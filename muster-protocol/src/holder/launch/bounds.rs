use std::time::Duration;

use muster_domain::value::PositiveDuration;

/// Longest a launched holder may take to accept its supervisor and send its
/// screen, and longest an adopted one may take to send its screen.
pub const HOLDER_READY_TIMEOUT: PositiveDuration =
    match PositiveDuration::try_new(Duration::from_secs(5)) {
        Ok(timeout) => timeout,
        Err(_) => panic!("the holder ready timeout must be positive"),
    };
/// Longest the runner spends stopping a holder it gave up on.
pub const ABANDON_TIMEOUT: Duration = Duration::from_secs(2);
