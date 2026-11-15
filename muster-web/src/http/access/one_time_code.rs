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
/// good for [`CODE_LIFETIME`], and gone at its first right use, so it cannot
/// be replayed. A wrong guess leaves it pending: at 256 bits a guess buys
/// nothing, and using the code up on one would let any local process, or a
/// web page loading an image from the view, deny the browser its open.
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

    /// Whether `candidate` is the pending code, within its lifetime at
    /// `now`; a match uses it up, and so does finding it expired.
    pub fn redeem(&self, candidate: &str, now: Instant) -> bool {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let Some((code, minted)) = pending.as_ref() else {
            return false;
        };
        if now.saturating_duration_since(*minted) >= CODE_LIFETIME {
            *pending = None;
            return false;
        }
        let matched = code.matches(candidate);
        if matched {
            *pending = None;
        }
        matched
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
    /// one does not, and a wrong guess leaves the right code usable.
    #[test]
    fn a_code_redeems_once_within_its_lifetime() {
        let codes = OneTimeCode::default();
        let minted = Instant::now();

        let code = codes.mint(minted).unwrap();
        assert!(codes.redeem(&code, minted + Duration::from_secs(1)));
        assert!(!codes.redeem(&code, minted + Duration::from_secs(1)));

        let code = codes.mint(minted).unwrap();
        assert!(!codes.redeem(&code, minted + CODE_LIFETIME));
        assert!(!codes.redeem(&code, minted), "an expired code is gone");

        let code = codes.mint(minted).unwrap();
        assert!(!codes.redeem(&"0".repeat(64), minted));
        assert!(
            codes.redeem(&code, minted),
            "a wrong guess leaves it usable"
        );
        assert!(!codes.redeem(&code, minted), "a reused code is refused");

        assert!(!OneTimeCode::default().redeem(&code, minted));
    }
}
