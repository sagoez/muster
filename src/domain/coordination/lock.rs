use std::time::Duration;

use getset::Getters;
use nutype::nutype;
use typed_builder::TypedBuilder;

use crate::domain::coordination::Author;

/// The time-to-live a lock takes when the agent names none.
pub const LOCK_TTL_DEFAULT: Duration = Duration::from_secs(5 * 60);
/// The longest time-to-live a lock may take.
pub const LOCK_TTL_MAX: Duration = Duration::from_secs(60 * 60);
/// The longest lock name, in characters.
const LOCK_NAME_MAX_CHARS: usize = 64;
/// The longest note a holder may leave on a lock, in characters.
const LOCK_NOTE_MAX_CHARS: usize = 200;
/// Punctuation a lock name may hold besides lowercase letters and digits.
const LOCK_NAME_PUNCTUATION: &[char] = &['.', '_', '-'];

/// Whether `name` holds only lowercase ASCII letters, digits and `._-`.
fn is_lock_name(name: &str) -> bool {
    name.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || LOCK_NAME_PUNCTUATION.contains(&c))
}

/// Whether `note` holds no control character, so it prints as one line.
fn is_one_line(note: &str) -> bool {
    !note.chars().any(char::is_control)
}

/// Whether `ttl` is a whole second or more and at most [`LOCK_TTL_MAX`].
const fn is_lock_ttl(ttl: &Duration) -> bool {
    ttl.as_secs() >= 1 && ttl.as_secs() <= LOCK_TTL_MAX.as_secs()
}

/// The name of a project's lock, such as `migrations` or `db-reset`: trimmed,
/// 1 to 64 characters of `[a-z0-9._-]`.
#[nutype(
    sanitize(trim),
    validate(not_empty, len_char_max = LOCK_NAME_MAX_CHARS, predicate = is_lock_name),
    derive(Debug, Clone, PartialEq, Eq, Hash, AsRef, Display)
)]
pub struct LockName(String);

/// What a holder says it holds a lock for: trimmed, one line, at most 200
/// characters.
#[nutype(
    sanitize(trim),
    validate(not_empty, len_char_max = LOCK_NOTE_MAX_CHARS, predicate = is_one_line),
    derive(Debug, Clone, PartialEq, Eq, AsRef, Display)
)]
pub struct LockNote(String);

/// How long a lock holds before it frees itself: at least a second, at most
/// [`LOCK_TTL_MAX`].
#[nutype(
    const_fn,
    validate(predicate = is_lock_ttl),
    derive(Debug, Clone, Copy, PartialEq, Eq, AsRef)
)]
pub struct LockTtl(Duration);

impl LockTtl {
    /// The time-to-live of [`LOCK_TTL_DEFAULT`], checked at compile time.
    pub const DEFAULT: Self = match Self::try_new(LOCK_TTL_DEFAULT) {
        Ok(ttl) => ttl,
        Err(_) => panic!("the default lock ttl is out of range"),
    };
}

/// A named lock one agent holds on a project until it releases it or it
/// expires.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Lock {
    /// The lock's name.
    name: LockName,
    /// Who holds it.
    owner: Author,
    /// What the holder said it holds it for.
    note: Option<LockNote>,
    /// Unix epoch seconds after which the lock has expired.
    expires_at: u64,
}

impl Lock {
    /// Whether the lock still holds at `now` (Unix seconds).
    #[must_use]
    pub fn is_live(&self, now: u64) -> bool {
        self.expires_at >= now
    }

    /// How long the lock has left at `now`; zero once it has expired.
    #[must_use]
    pub fn expires_in(&self, now: u64) -> Duration {
        Duration::from_secs(self.expires_at.saturating_sub(now))
    }
}

/// What an attempt to take a lock came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LockAttempt {
    /// The caller holds the lock, newly or renewed.
    Acquired {
        /// How long it holds before it expires.
        expires_in: Duration,
    },
    /// Someone else holds it.
    HeldBy {
        /// Who holds it.
        owner: Author,
        /// What they said they hold it for.
        note: Option<LockNote>,
        /// How long it has left.
        expires_in: Duration,
    },
}

/// What releasing a lock came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LockRelease {
    /// The caller held the lock and it is gone.
    Released,
    /// Someone else holds it, so it stays.
    HeldBy {
        /// Who holds it.
        owner: Author,
    },
    /// Nobody holds it.
    NotHeld,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A name is trimmed lowercase `[a-z0-9._-]`, 1 to 64 characters.
    #[test]
    fn a_lock_name_is_validated() {
        assert_eq!(
            LockName::try_new("  db-reset ").unwrap().as_ref(),
            "db-reset"
        );
        assert!(LockName::try_new("migrations.v2_a").is_ok());
        assert!(LockName::try_new("").is_err());
        assert!(LockName::try_new("DB").is_err());
        assert!(LockName::try_new("db reset").is_err());
        assert!(LockName::try_new("a".repeat(LOCK_NAME_MAX_CHARS)).is_ok());
        assert!(LockName::try_new("a".repeat(LOCK_NAME_MAX_CHARS + 1)).is_err());
    }

    /// A note is one bounded line.
    #[test]
    fn a_lock_note_is_one_bounded_line() {
        assert!(LockNote::try_new("running migrations").is_ok());
        assert!(LockNote::try_new("two\nlines").is_err());
        assert!(LockNote::try_new("\u{1b}[31mred").is_err());
        assert!(LockNote::try_new("x".repeat(LOCK_NOTE_MAX_CHARS + 1)).is_err());
    }

    /// A ttl is at least a second and at most the maximum.
    #[test]
    fn a_ttl_is_bounded() {
        assert!(LockTtl::try_new(Duration::ZERO).is_err());
        assert!(LockTtl::try_new(LOCK_TTL_MAX).is_ok());
        assert!(LockTtl::try_new(LOCK_TTL_MAX + Duration::from_secs(1)).is_err());
        assert_eq!(*LockTtl::DEFAULT.as_ref(), LOCK_TTL_DEFAULT);
    }

    /// A lock is live through its last second and reports what it has left.
    #[test]
    fn a_lock_expires_after_its_last_second() {
        let lock = Lock::builder()
            .name(LockName::try_new("migrations").unwrap())
            .owner(Author::agent("claude"))
            .note(None)
            .expires_at(100)
            .build();
        assert!(lock.is_live(100));
        assert!(!lock.is_live(101));
        assert_eq!(lock.expires_in(40), Duration::from_secs(60));
        assert_eq!(lock.expires_in(200), Duration::ZERO);
    }
}
