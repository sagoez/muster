//! Locking a mutex whatever a panicking holder left in it.

use std::sync::{Mutex, MutexGuard, PoisonError};

/// A mutex taken even when a thread panicked holding it: muster's shared
/// state stays usable after one thread panics, so its guard is taken either
/// way.
pub trait Locked<T: ?Sized> {
    /// Locks, recovering the guard from a poisoned lock.
    fn locked(&self) -> MutexGuard<'_, T>;
}

impl<T: ?Sized> Locked<T> for Mutex<T> {
    fn locked(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, thread};

    use super::*;

    #[test]
    fn a_lock_poisoned_by_a_panic_still_hands_out_its_value() {
        let shared = Arc::new(Mutex::new(1));
        let holder = Arc::clone(&shared);
        let _ = thread::spawn(move || {
            let mut value = holder.lock().unwrap();
            *value = 2;
            panic!("poisons the lock");
        })
        .join();

        assert!(shared.is_poisoned());
        assert_eq!(*shared.locked(), 2);
    }
}
