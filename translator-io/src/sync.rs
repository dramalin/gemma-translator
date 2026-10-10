use std::sync::{Mutex, MutexGuard, PoisonError};

/// Locks `mutex`, ignoring poisoning: the guarded data stays usable after a
/// panic on another thread (a poisoned audio buffer must not kill the UI).
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
