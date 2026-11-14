use std::{
    fmt::{self, Debug, Formatter},
    io,
    sync::{Mutex, PoisonError},
    time::{Duration, Instant},
};

use super::open_code::OpenCode;

/// How long a code the browser was opened with stays good.
pub const CODE_LIFETIME: Duration = Duration::from_secs(10);

/// The code the browser is opened with, held only in memory: minted once,
/// good for [`CODE_LIFETIME`], and gone at its first use, right or wrong, so
/// it can be neither replayed nor guessed at.
#[derive(Default)]
pub struct OneTimeCode {
    /// The code waiting to be used, and when it was minted.
    pending: Mutex<Option<(OpenCode, Instant)>>,
}

impl OneTimeCode {
    /// Mints a fresh code at `now`, replacing any still pending, and returns
    /// its text.
    ///
    /// # Errors
    /// Returns the failure to read the random source.
    pub fn mint(&self, now: Instant) -> io::Result<String> {
        let code = OpenCode::generate()?;
        let secret = code.secret().to_string();
        *self.pending.lock().unwrap_or_else(PoisonError::into_inner) = Some((code, now));
        Ok(secret)
    }

    /// Uses the pending code up and says whether `candidate` was it, within
    /// its lifetime at `now`.
    pub fn redeem(&self, candidate: &str, now: Instant) -> bool {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .is_some_and(|(code, minted)| {
                code.matches(candidate) && now.saturating_duration_since(minted) < CODE_LIFETIME
            })
    }
}

impl Debug for OneTimeCode {
    /// Names the type only, never the code.
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OneTimeCode")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A right code redeems once; a second use, a wrong code or an expired
    /// one does not, and a wrong guess uses the code up.
    #[test]
    fn a_code_redeems_once_within_its_lifetime() {
        let codes = OneTimeCode::default();
        let minted = Instant::now();

        let code = codes.mint(minted).unwrap();
        assert!(codes.redeem(&code, minted + Duration::from_secs(1)));
        assert!(!codes.redeem(&code, minted + Duration::from_secs(1)));

        let code = codes.mint(minted).unwrap();
        assert!(!codes.redeem(&code, minted + CODE_LIFETIME));

        let code = codes.mint(minted).unwrap();
        assert!(!codes.redeem(&"0".repeat(64), minted));
        assert!(!codes.redeem(&code, minted), "a wrong guess uses it up");

        assert!(!OneTimeCode::default().redeem(&code, minted));
    }
}
