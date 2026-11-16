use std::time::Duration;

use muster_domain::value::PositiveDuration;

/// Longest a launched holder may take to accept its supervisor and send its
/// screen, and longest an adopted one may take to send its screen.
pub const HOLDER_READY_TIMEOUT: PositiveDuration =
    match PositiveDuration::try_new(Duration::from_secs(5)) {
        Ok(timeout) => timeout,
        Err(_) => panic!("the holder ready timeout must be positive"),
    };
/// How often a holder looks whether its socket is still where it bound it,
/// so one a tmp cleaner removed is bound again within this.
pub const SOCKET_CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// Socket checks a wait on a holder outlasts, so a holder whose socket was
/// removed just before is reached once it binds it again.
const CHECKS_OUTWAITED: u32 = 3;
/// Longest the runner spends reaching a holder to stop it or collect its
/// exit: comfortably past a socket check, so a holder in the middle of
/// binding its socket again is still reached.
pub const ABANDON_TIMEOUT: Duration = SOCKET_CHECK_INTERVAL.saturating_mul(CHECKS_OUTWAITED);

#[cfg(test)]
mod tests {
    use super::*;

    /// Reaching a holder to stop it or collect its exit outlasts more than
    /// one socket check, so a holder binding its socket again is reached.
    #[test]
    fn reaching_a_holder_outwaits_its_socket_checks() {
        assert!(ABANDON_TIMEOUT > SOCKET_CHECK_INTERVAL.saturating_mul(2));
    }
}
